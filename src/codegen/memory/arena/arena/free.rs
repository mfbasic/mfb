//! plan-176-P, plan-183-C: `lower_arena_free` — returning a chunk to its region.

use super::layout::*;
use super::small::*;
use super::span::*;
use super::*;

/// `arena_free(x0 = ptr, x1 = size)` — return a compiler-sized allocation to the
/// arena. The chunk's region follows from its address: a slot of the small pages is
/// freed inline (`small.rs`), a medium or huge chunk by `_mfb_arena_span_free`, which
/// knows its true size from its tag. Every freed byte is zeroed before the chunk can
/// be handed out again, and no allocator metadata is written into it (plan-183-C).
/// A pointer that is not a chunk this arena handed out — a repeated free, an address
/// outside its slice — changes nothing and counts as a skipped double free. `size`
/// is normalized exactly as `arena_alloc` normalized it and is used for the `--debug`
/// counters. Vreg-allocated; clobbers the caller-saved registers.
/// `perf` (plan-130-B): bracket the body with the `mfb_free` perf span.
pub(crate) fn lower_arena_free(perf: bool, debug_arena: bool) -> CodeFunction {
    let symbol = ARENA_FREE_SYMBOL;
    let mut vregs = Vregs::new();
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let ptr = vregs.next();
    let size = vregs.next();
    let sc = vregs.next();
    let freed = vregs.next();
    let t = vregs.next();
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&ptr, abi::c_arg(0)),
        abi::move_register(&size, abi::c_arg(1)),
        abi::compare_immediate(&size, "0"),
        abi::branch_ne("arena_free_size_nonzero"),
        abi::move_immediate(&size, "Integer", "1"),
        abi::label("arena_free_size_nonzero"),
        abi::add_immediate(&size, &size, (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&t, "Integer", &not_15),
        abi::and_registers(&size, &size, &t),
    ];
    let mut dbg_slot: Option<VirtualRegister> = None;
    if debug_arena {
        let slot = vregs.next();
        emit_debug_arena_slot(
            symbol,
            &slot,
            &mut instructions,
            &mut relocations,
            &mut vregs,
        );
        emit_debug_arena_bump(
            symbol,
            &slot,
            COUNTER_FREE_CALLS,
            "free_calls",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            symbol,
            &slot,
            COUNTER_FREE_BYTES,
            &size,
            "free_bytes",
            &mut instructions,
            &mut vregs,
        );
        dbg_slot = Some(slot);
    }
    if perf {
        emit_perf_arena_call(
            "perf.start",
            symbol,
            PERF_NAME_MFB_FREE_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_PAGES),
        abi::compare_immediate(&t, "0"),
        abi::branch_eq("arena_free_skip"),
        abi::compare_registers(&ptr, &t),
        abi::branch_lo("arena_free_skip"),
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_END),
        abi::compare_registers(&ptr, &t),
        abi::branch_lo("arena_free_small"),
        abi::move_register(abi::c_arg(0), &ptr),
        abi::branch_link(ARENA_SPAN_FREE_SYMBOL),
        abi::move_register(&freed, abi::return_register()),
        abi::compare_immediate(&freed, "0"),
        abi::branch_eq("arena_free_skip"),
        abi::branch("arena_free_freed"),
        abi::label("arena_free_small"),
    ]);
    relocations.push(internal_branch(symbol, ARENA_SPAN_FREE_SYMBOL));
    emit_small_free(
        symbol,
        &ptr,
        &sc,
        &freed,
        "arena_free_skip",
        &dbg_slot,
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    instructions.push(abi::label("arena_free_freed"));
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_live_sub(
            symbol,
            slot,
            &size,
            "free_live",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.extend([
        abi::branch("arena_free_done"),
        abi::label("arena_free_skip"),
    ]);
    if let Some(slot) = &dbg_slot {
        emit_debug_arena_bump(
            symbol,
            slot,
            COUNTER_DOUBLE_FREE_SKIPS,
            "double_free",
            &mut instructions,
            &mut vregs,
        );
    }
    instructions.push(abi::label("arena_free_done"));
    if perf {
        emit_perf_arena_call(
            "perf.end",
            symbol,
            PERF_NAME_MFB_FREE_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.push(abi::return_());
    finalize_vreg_helper(
        "runtime.arena_free",
        symbol,
        "Nothing",
        instructions,
        relocations,
    )
}
