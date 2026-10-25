//! plan-176-P: the arena allocator (`lower_arena_alloc`) and the SIMD list allocator.
//! plan-183-C: `arena_alloc` dispatches to the slice's regions — the small pages
//! inline (`small.rs`), the medium and huge runs through their helpers (`span.rs`).

use super::layout::*;
use super::small::*;
use super::span::*;
use super::*;

/// `perf` (plan-130-B): bracket the body with the `mfb_alloc` perf span — true exactly
/// when the `--debug` perf section is active for the module.
/// `debug_arena` (plan-130-C): count this helper's events in the `--debug` arena
/// registry — true exactly when the arena section is active for the module.
///
/// Register contract (allocator-06): the standard runtime-helper one — every
/// caller-saved integer register is clobbered, callee-saved ones are preserved by the
/// frame. The fast small path makes no call; a new page, a medium or huge request and
/// the first allocation (which lays the slice out) do.
pub(crate) fn lower_arena_alloc(perf: bool, debug_arena: bool) -> CodeFunction {
    let symbol = ARENA_ALLOC_SYMBOL;
    let not_15 = (!(ARENA_MIN_CHUNK - 1)).to_string();
    let mut vregs = Vregs::new();
    let size = vregs.next();
    let align = vregs.next();
    let sc = vregs.next();
    let ptr = vregs.next();
    let t = vregs.next();
    let mut relocations = Vec::new();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&size, abi::c_arg(0)),
        abi::move_register(&align, abi::c_arg(1)),
        // A zero alignment, or one that is not a power of two, is invalid.
        abi::compare_immediate(&align, "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate(&t, &align, 1),
        abi::and_registers(&t, &align, &t),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne("arena_alloc_invalid"),
        abi::move_immediate(&t, "Integer", &ARENA_MAX_ALIGN.to_string()),
        abi::compare_registers(&align, &t),
        abi::branch_hi("arena_alloc_invalid"),
    ];
    if perf {
        emit_perf_arena_call(
            "perf.start",
            symbol,
            PERF_NAME_MFB_ALLOC_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
    }
    instructions.extend([
        // Reject a request within ARENA_MIN_CHUNK of u64::MAX before the round-up
        // (allocator-02): it would wrap to a small size.
        abi::move_immediate(&t, "Integer", &(u64::MAX - ARENA_MIN_CHUNK).to_string()),
        abi::compare_registers(&size, &t),
        abi::branch_hi("arena_alloc_invalid"),
        abi::compare_immediate(&size, "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate(&size, "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::add_immediate(&size, &size, (ARENA_MIN_CHUNK - 1) as usize),
        abi::move_immediate(&t, "Integer", &not_15),
        abi::and_registers(&size, &size, &t),
    ]);
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
            COUNTER_ALLOC_CALLS,
            "alloc_calls",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_arena_add(
            symbol,
            &slot,
            COUNTER_ALLOC_BYTES,
            &size,
            "alloc_bytes",
            &mut instructions,
            &mut vregs,
        );
        dbg_slot = Some(slot);
    }
    // The slice is laid out at the arena's first allocation.
    emit_scalars(&sc, &mut instructions);
    instructions.extend([
        abi::load_u64(&t, &sc, ARENA_SC_SMALL_PAGES),
        abi::compare_immediate(&t, "0"),
        abi::branch_ne("arena_alloc_laid_out"),
        abi::branch_link(ARENA_GEOMETRY_SYMBOL),
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ne("arena_alloc_oom"),
        abi::label("arena_alloc_laid_out"),
        // An alignment above 16 is served by the huge region, whose chunks are page
        // aligned.
        abi::compare_immediate(&align, &ARENA_MIN_CHUNK.to_string()),
        abi::branch_hi("arena_alloc_huge_round"),
        abi::compare_immediate(&size, &ARENA_SMALL_MAX.to_string()),
        abi::branch_hi("arena_alloc_not_small"),
    ]);
    relocations.push(internal_branch(symbol, ARENA_GEOMETRY_SYMBOL));
    emit_small_alloc(
        symbol,
        &size,
        &sc,
        &ptr,
        "arena_alloc_oom",
        &mut instructions,
        &mut relocations,
        &mut vregs,
    );
    emit_debug_alloc_hit(
        &dbg_slot,
        COUNTER_HIT_SMALL,
        &size,
        "hit_small",
        &mut instructions,
        &mut vregs,
    );
    instructions.extend([
        abi::branch("arena_alloc_ok"),
        abi::label("arena_alloc_not_small"),
        abi::move_immediate(&t, "Integer", &ARENA_HUGE_MIN.to_string()),
        abi::compare_registers(&size, &t),
        abi::branch_lo("arena_alloc_medium"),
        abi::branch("arena_alloc_huge_round"),
        abi::label("arena_alloc_medium"),
        abi::move_register(abi::c_arg(0), &size),
        abi::branch_link(ARENA_MEDIUM_ALLOC_SYMBOL),
        abi::move_register(&ptr, abi::return_register()),
        abi::move_register(&t, abi::mfb_return(1)),
        abi::compare_immediate(&ptr, "0"),
        abi::branch_eq("arena_alloc_oom"),
    ]);
    relocations.push(internal_branch(symbol, ARENA_MEDIUM_ALLOC_SYMBOL));
    if dbg_slot.is_some() {
        instructions.extend([
            abi::compare_immediate(&t, "0"),
            abi::branch_ne("arena_alloc_medium_front"),
        ]);
        emit_debug_alloc_hit(
            &dbg_slot,
            COUNTER_HIT_MEDIUM_RUN,
            &size,
            "hit_medium_run",
            &mut instructions,
            &mut vregs,
        );
        instructions.extend([
            abi::branch("arena_alloc_ok"),
            abi::label("arena_alloc_medium_front"),
        ]);
        emit_debug_alloc_hit(
            &dbg_slot,
            COUNTER_HIT_MEDIUM_FRONTIER,
            &size,
            "hit_medium_frontier",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_grow_sample(&dbg_slot, &mut instructions, &mut relocations);
    }
    instructions.extend([
        abi::branch("arena_alloc_ok"),
        // Huge: whole 16 KiB pages (the size overflow-checked).
        abi::label("arena_alloc_huge_round"),
    ]);
    // `size` stays the normalized request (what the counters and the free see); the
    // helper is asked for whole pages.
    emit_round_up(&ptr, &size, ARENA_HUGE_PAGE, &t, &mut instructions);
    instructions.extend([
        abi::compare_registers(&ptr, &size),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_register(abi::c_arg(0), &ptr),
        abi::branch_link(ARENA_HUGE_ALLOC_SYMBOL),
        abi::move_register(&ptr, abi::return_register()),
        abi::move_register(&t, abi::mfb_return(1)),
        abi::compare_immediate(&ptr, "0"),
        abi::branch_eq("arena_alloc_oom"),
    ]);
    relocations.push(internal_branch(symbol, ARENA_HUGE_ALLOC_SYMBOL));
    if dbg_slot.is_some() {
        instructions.extend([
            abi::compare_immediate(&t, "0"),
            abi::branch_ne("arena_alloc_huge_front"),
        ]);
        emit_debug_alloc_hit(
            &dbg_slot,
            COUNTER_HIT_HUGE_RUN,
            &size,
            "hit_huge_run",
            &mut instructions,
            &mut vregs,
        );
        instructions.extend([
            abi::branch("arena_alloc_ok"),
            abi::label("arena_alloc_huge_front"),
        ]);
        emit_debug_alloc_hit(
            &dbg_slot,
            COUNTER_HIT_HUGE_FRONTIER,
            &size,
            "hit_huge_frontier",
            &mut instructions,
            &mut vregs,
        );
        emit_debug_grow_sample(&dbg_slot, &mut instructions, &mut relocations);
    }
    instructions.extend([
        abi::label("arena_alloc_ok"),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register(abi::mfb_return(1), &ptr),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            crate::codegen::registry::runtime_error("ErrInvalidArgument")
                .expect("errorCode name")
                .0,
        ),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::branch("arena_alloc_ret"),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            crate::codegen::registry::runtime_error("ErrOutOfMemory")
                .expect("errorCode name")
                .0,
        ),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::label("arena_alloc_ret"),
    ]);
    if let Some(slot) = &dbg_slot {
        emit_debug_zero_check(slot, &size, &mut instructions, &mut relocations, &mut vregs);
    }
    if perf {
        let saved_tag = vregs.next();
        let saved_ptr = vregs.next();
        instructions.push(abi::move_register(&saved_tag, abi::return_register()));
        instructions.push(abi::move_register(&saved_ptr, abi::mfb_return(1)));
        emit_perf_arena_call(
            "perf.end",
            symbol,
            PERF_NAME_MFB_ALLOC_SYMBOL,
            &mut instructions,
            &mut relocations,
        );
        instructions.push(abi::move_register(abi::return_register(), &saved_tag));
        instructions.push(abi::move_register(abi::mfb_return(1), &saved_ptr));
    }
    instructions.push(abi::return_());
    finalize_vreg_helper(
        "runtime.arena_alloc",
        symbol,
        "Pointer",
        instructions,
        relocations,
    )
}

/// plan-133-C, plan-183-C: a region's frontier extended — sample the arena's memory
/// series (the helper counts the grow). The registry slot is the only live value the
/// call needs; every other one is a vreg the allocator spills.
pub(crate) fn emit_debug_grow_sample(
    slot: &Option<VirtualRegister>,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) {
    if let Some(slot) = slot {
        instructions.extend([
            abi::move_register(abi::c_arg(0), slot),
            abi::branch_link(DEBUG_ARENA_SAMPLE_SYMBOL),
        ]);
        relocations.push(internal_branch(
            ARENA_ALLOC_SYMBOL,
            DEBUG_ARENA_SAMPLE_SYMBOL,
        ));
    }
}

/// plan-183-A: in a `--debug` build started with `MFB_ARENA_CHECK_ZERO` set, count the
/// successful allocation in `zero_checks` and, when any word of the chunk past its first
/// [`ARENA_ZERO_CHECK_SKIP`] bytes is non-zero, in `zero_violations`. The result (tag
/// and pointer) is copied into vregs first and put back last, so the scan cannot be
/// handed a result register. Call-free; reads only the chunk and the switch word.
fn emit_debug_zero_check(
    slot: &VirtualRegister,
    size: &VirtualRegister,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) {
    let tag = vregs.next();
    let ptr = vregs.next();
    let flag = vregs.next();
    let cursor = vregs.next();
    let end = vregs.next();
    let word = vregs.next();
    instructions.extend([
        abi::move_register(&tag, abi::return_register()),
        abi::move_register(&ptr, abi::mfb_return(1)),
        abi::compare_immediate(&tag, RESULT_OK_TAG),
        abi::branch_ne("arena_alloc_zero_check_done"),
        abi::compare_immediate(slot, "0"),
        abi::branch_eq("arena_alloc_zero_check_done"),
    ]);
    push_symbol_address(
        ARENA_ALLOC_SYMBOL,
        crate::codegen::debug::arena::ARENA_CHECK_ZERO_SYMBOL,
        &flag,
        instructions,
        relocations,
    );
    instructions.extend([
        abi::load_u64(&flag, &flag, 0),
        abi::compare_immediate(&flag, "0"),
        abi::branch_eq("arena_alloc_zero_check_done"),
    ]);
    emit_debug_arena_bump(
        ARENA_ALLOC_SYMBOL,
        slot,
        COUNTER_ZERO_CHECKS,
        "zero_checks",
        instructions,
        vregs,
    );
    instructions.extend([
        abi::add_immediate(&cursor, &ptr, ARENA_ZERO_CHECK_SKIP),
        abi::add_registers(&end, &ptr, size),
        abi::label("arena_alloc_zero_check_loop"),
        abi::compare_registers(&cursor, &end),
        abi::branch_lo("arena_alloc_zero_check_word"),
        abi::branch("arena_alloc_zero_check_done"),
        abi::label("arena_alloc_zero_check_word"),
        abi::load_u64(&word, &cursor, 0),
        abi::add_immediate(&cursor, &cursor, 8),
        abi::compare_immediate(&word, "0"),
        abi::branch_eq("arena_alloc_zero_check_loop"),
    ]);
    emit_debug_arena_bump(
        ARENA_ALLOC_SYMBOL,
        slot,
        COUNTER_ZERO_VIOLATIONS,
        "zero_violations",
        instructions,
        vregs,
    );
    instructions.extend([
        abi::label("arena_alloc_zero_check_done"),
        abi::move_register(abi::return_register(), &tag),
        abi::move_register(abi::mfb_return(1), &ptr),
    ]);
}

/// `_mfb_simd_alloc_list(x0 = count, x1 = valueTypeCode) -> x0 = base` —
/// allocate a tight homogeneous numeric `List` (plan-01-simd §4.3). The data
/// region is `count` contiguous 8-byte lanes at `base + 40 + count*40`. Returns
/// `0` if the arena allocation fails (the caller raises the allocation error).
///
/// Calls `_mfb_arena_alloc`, whose clobber set is wide (`x0,x1,x9,x10,x14,x15,
/// x16,x20-x28`); `count` and `valueTypeCode` are spilled across the call and
/// reloaded. After the call there are no further calls, so the header/entry
/// writes use scratch GPRs freely.
pub(crate) fn lower_simd_alloc_list() -> CodeFunction {
    let mut vregs = Vregs::new();
    // count/typeCode are live across the arena_alloc call (ALL_INT clobber), so
    // they spill — the old hand frame's COUNT/TYPE slots, now allocator-managed.
    let count = vregs.next();
    let type_code = vregs.next();
    let stride = vregs.next();
    let mut instructions = vec![
        abi::label("entry"),
        abi::move_register(&count, abi::c_arg(0)),
        abi::move_register(&type_code, abi::c_arg(1)),
        // alloc size = COLLECTION_HEADER_SIZE + count*(stride + 8) (lookup + data).
        // Every list this helper builds has an 8-byte fixed-width element
        // (Integer/Float/Fixed/Money), so its stride is uniform — kind 2 drops
        // the lookup array and the block becomes HEADER + count*8 (plan-57-D).
        abi::move_immediate(
            &stride,
            "Integer",
            &(list_entry_stride(&ParameterType::Integer) + 8).to_string(),
        ),
        abi::multiply_registers(abi::c_arg(0), &count, &stride),
        abi::add_immediate(abi::c_arg(0), abi::c_arg(0), COLLECTION_HEADER_SIZE),
        abi::move_immediate(abi::c_arg(1), "Integer", "8"),
        abi::branch_link(ARENA_ALLOC_SYMBOL),
        // x0 = result tag, x1 = pointer. Return x0 = base, x1 = status (0 = ok,
        // else the arena error tag) so the caller can raise the allocation error.
        abi::compare_immediate(abi::return_register(), RESULT_OK_TAG),
        abi::branch_eq("simd_alloc_ok"),
        abi::move_register(abi::mfb_return(1), abi::return_register()),
        abi::move_immediate(abi::return_register(), "Integer", "0"),
        abi::branch("simd_alloc_ret"),
        abi::label("simd_alloc_ok"),
    ];
    // A `List OF Integer` is a fixed-width (kind 2) list (plan-57-D): its kind byte
    // is non-zero, so KEY_TYPE takes a zero register of its own, and it has no
    // lookup array to fill. (The kind-0 layout this once also emitted — one shared
    // zero register, then an identity-mapped entry loop — is gone with that layout;
    // the pins below say so if it ever comes back.) The three trailing vregs are
    // still minted so the numbering of everything after them is unchanged.
    debug_assert_ne!(list_block_kind(&ParameterType::Integer), 0);
    debug_assert_eq!(list_entry_stride(&ParameterType::Integer), 0);
    let base = vregs.next();
    let scratch = vregs.next();
    let kind_zero = vregs.next();
    let data_len = vregs.next();
    let [_, _, _] = std::array::from_fn(|_| vregs.next());
    instructions.push(abi::move_immediate(&kind_zero, "Integer", "0"));
    instructions.extend([
        abi::move_register(&base, abi::mfb_return(1)),
        // Header: kind, keyType=0, valueType=typeCode, flagsVersion=1.
        abi::move_immediate(
            &scratch,
            "Integer",
            &list_block_kind(&ParameterType::Integer).to_string(),
        ),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_KIND),
        abi::store_u8(&kind_zero, &base, COLLECTION_OFFSET_KEY_TYPE),
        abi::store_u8(&type_code, &base, COLLECTION_OFFSET_VALUE_TYPE),
        abi::move_immediate(&scratch, "Integer", "1"),
        abi::store_u8(&scratch, &base, COLLECTION_OFFSET_FLAGS_VERSION),
        // count, capacity = count; dataLength, dataCapacity = count*8.
        abi::store_u64(&count, &base, COLLECTION_OFFSET_COUNT),
        abi::store_u64(&count, &base, COLLECTION_OFFSET_CAPACITY),
        abi::shift_left_immediate(&data_len, &count, 3),
        abi::store_u64(&data_len, &base, COLLECTION_OFFSET_DATA_LENGTH),
        abi::store_u64(&data_len, &base, COLLECTION_OFFSET_DATA_CAPACITY),
    ]);
    instructions.extend([
        abi::move_register(abi::return_register(), &base),
        abi::move_immediate(abi::mfb_return(1), "Integer", "0"),
        abi::label("simd_alloc_ret"),
        abi::return_(),
    ]);
    finalize_vreg_helper(
        "runtime.simd_alloc_list",
        SIMD_ALLOC_LIST_SYMBOL,
        "Pointer",
        instructions,
        vec![internal_branch(SIMD_ALLOC_LIST_SYMBOL, ARENA_ALLOC_SYMBOL)],
    )
}
