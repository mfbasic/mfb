//! plan-176-O: the `--debug` arena sampler (`lower_sample`). The counter tables, the registry and
//! the per-allocation emitters live in `codegen/debug/arena/mod.rs`.

use super::*;

/// `_mfb_debug_arena_sample(slot)` (plan-133-C): count one grow (plan-183-C: a region's
/// frontier extended) in the slot's `grows`, write its sample into the entry after the
/// kept ones, then keep it when that count has reached `until`.
/// Keeping the 256th sample halves the series in place — the even entries, then the one
/// just kept — and doubles the stride. An unkept grow sets `pending`, so the report shows
/// the latest grow as the last sample either way.
pub(super) fn lower_sample(
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
    let committed = vregs.next();
    let live = vregs.next();
    let grows = vregs.next();
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
        abi::load_u64(&committed, &slot, COUNTER_COMMITTED_BYTES),
        abi::load_u64(&live, &slot, COUNTER_LIVE_BYTES),
        abi::store_u64(&now, &entry, SERIES_FIELDS[0].1),
        abi::store_u64(&committed, &entry, SERIES_FIELDS[1].1),
        abi::store_u64(&live, &entry, SERIES_FIELDS[2].1),
        abi::store_u64(&rss, &entry, SERIES_FIELDS[3].1),
        // Keep it once the grow count reaches `until` (0 before the first grow).
        abi::load_u64(&grows, &slot, COUNTER_GROWS),
        abi::add_immediate(&grows, &grows, 1),
        abi::store_u64(&grows, &slot, COUNTER_GROWS),
        abi::load_u64(&until, &slot, SERIES_UNTIL_OFFSET),
        abi::compare_registers(&grows, &until),
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
        abi::add_registers(&until, &grows, &stride),
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
