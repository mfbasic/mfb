//! plan-176-O: the `--debug` arena report itself — one line per arena slot and per sample series
//! (`lower_report`). The counter tables, the registry and the per-allocation emitters live in
//! `codegen/debug/arena/mod.rs`.

use super::*;

/// `arena.<index><suffix><value>\n` in the helper's window: right to left, the value
/// digits, the suffix object (`.maps ` …), the index digits, `arena.`.
#[allow(clippy::too_many_arguments)]
fn emit_slot_line(
    symbol: &str,
    index: impl Into<abi::Reg>,
    suffix_symbol: &str,
    value: impl Into<abi::Reg>,
    tag: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let index: abi::Reg = index.into();
    let value: abi::Reg = value.into();
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
    index: impl Into<abi::Reg>,
    sample: impl Into<abi::Reg>,
    suffix_symbol: &str,
    value: impl Into<abi::Reg>,
    tag: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
    vregs: &mut Vregs,
) -> Result<(), String> {
    let index: abi::Reg = index.into();
    let sample: abi::Reg = sample.into();
    let value: abi::Reg = value.into();
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

pub(super) fn lower_report(
    slices: u64,
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
    let thread_block = "debug_arena_kind_thread_block";
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
    // plan-183-B: the reservation — the build's slice count and the slice size the
    // startup reservation settled on; plan-183-E: the bytes it reserved.
    let slice_count = vregs.next();
    let slice_size = vregs.next();
    let reserved = vregs.next();
    instructions.push(abi::move_immediate(
        &slice_count,
        "Integer",
        &slices.to_string(),
    ));
    push_symbol_address(
        symbol,
        crate::codegen::runtime::arena_cap::ARENA_SLICE_SIZE_SYMBOL,
        &slice_size,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&slice_size, &slice_size, 0));
    push_symbol_address(
        symbol,
        crate::codegen::runtime::arena_cap::ARENA_RESERVED_BYTES_SYMBOL,
        &reserved,
        &mut instructions,
        &mut relocations,
    );
    instructions.push(abi::load_u64(&reserved, &reserved, 0));
    for (key, value, tag) in [
        (ARENA_COUNT_KEY_SYMBOL, &count, "count"),
        (ARENA_OVERFLOW_KEY_SYMBOL, &overflow, "overflow"),
        (ARENA_SLICES_KEY_SYMBOL, &slice_count, "slices"),
        (ARENA_SLICE_SIZE_KEY_SYMBOL, &slice_size, "slice_size"),
        (ARENA_RESERVATION_KEY_SYMBOL, &reserved, "reservation_bytes"),
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
        abi::compare_immediate(&kind, ARENA_KIND_THREAD_BLOCK),
        abi::branch_eq(thread_block),
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
    instructions.extend([abi::branch(line), abi::label(thread_block)]);
    push_symbol_address(
        symbol,
        ARENA_TOKEN_THREAD_BLOCK_SYMBOL,
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
