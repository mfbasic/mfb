//! plan-176-V: the dash and join uploads. The shared constants, layouts and imports live in
//! `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// plan-163-B: copy this item's dash words into the frame buffer's dash region, and name
/// the slice in its item block — `strokeStyle.z` the first word, `strokeStyle.w` the word
/// count (`emit_join_upload` zeroed both, which the shader reads as a solid stroke).
///
/// The words are float32 bit patterns: the phase (header slot `HEADER_DASH_PHASE`), then
/// the record's dash tail — the entry count, the period, each entry's end, and the kind's
/// perimeter data (`helper_dashes.rs`) — found at `slot1 − stops·5 − dashes`, right before
/// the gradient stops. Floats rather than 16.16: the pattern's lengths and an outline's
/// perimeter positions are compared, never added to a coordinate, and fp32 holds a
/// thousand-pixel perimeter to 6e-5 px. A frame whose words would pass the region's cap
/// is declined by the predicate, so the over-cap arm is unreachable and leaves the stroke
/// solid rather than write past the region.
pub(super) fn emit_dash_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_dash_cursor: usize,
) {
    let head = builder.label("vk_dash_head");
    let done = builder.label("vk_dash_done");
    let copy = builder.label("vk_dash_copy");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let cursor = abi::SCRATCH[5];
    let target = abi::SCRATCH[6];
    let style = off_item + ITEM_OFFSET_STROKE_STYLE;

    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_DASH_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(count, "0"));
    builder.emit(abi::branch_le(&done));
    builder.emit(abi::load_u64(cursor, abi::stack_pointer(), off_dash_cursor));
    builder.emit(abi::add_registers(abi::SCRATCH[1], cursor, count));
    builder.emit(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &VULKAN_MAX_FRAME_DASH_WORDS.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[7]));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_dash_cursor,
    ));
    builder.emit(abi::store_u32(
        cursor,
        abi::stack_pointer(),
        style + ITEM_STROKE_DASH_BASE,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[7], count, 1));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        style + ITEM_STROKE_DASH_COUNT,
    ));
    // target = mapped + (VULKAN_DASH_BASE_WORDS + cursor) * 4.
    builder.emit(abi::add_immediate(
        abi::SCRATCH[1],
        cursor,
        VULKAN_DASH_BASE_WORDS,
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        2,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[1]));
    // The header again: the state load may have used the scratch bank.
    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_DASH_PHASE * 8,
    ));
    builder.emit(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[7], target, 0));
    builder.emit(abi::add_immediate(target, target, 4));
    // source = header + (slot1 - stops * 5 - count) * 8.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, 8));
    builder.emit(abi::float_convert_to_signed_x(source, abi::FP_SCRATCH[1]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[8], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    // plan-163-C: the path clip's edges sit between the dashes and the stops.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    builder.emit(abi::subtract_registers(source, source, count));
    builder.emit(abi::shift_left_immediate(source, source, 3));
    builder.emit(abi::add_registers(source, header, source));
    builder.emit(abi::move_immediate(index, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 0));
    builder.emit(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[7], target, 0));
    builder.emit(abi::add_immediate(target, target, 4));
    builder.emit(abi::add_immediate(source, source, 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
}

pub(super) fn emit_join_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_edge_cursor: usize,
) {
    let head = builder.label("vk_join_head");
    let done = builder.label("vk_join_done");
    let none = builder.label("vk_join_none");
    let copy = builder.label("vk_join_copy");
    let records = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let slots = abi::SCRATCH[6];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];
    let style = off_item + ITEM_OFFSET_STROKE_STYLE;

    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in [ITEM_STROKE_DASH_BASE, ITEM_STROKE_DASH_COUNT] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            style + word,
        ));
    }
    builder.emit(abi::load_u64(source, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_JOIN_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(records, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(records, "0"));
    builder.emit(abi::branch_le(&none));
    builder.emit(abi::shift_left_immediate(slots, records, 1));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], slots));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    // Unreachable: `__canvas_blockCost` counts two edges per record, and the pass walk
    // ends a pass before its total passes the region (bug-765). Kept because the
    // alternative is a write past the buffer; dropping the records draws the corners
    // round, not a wrong shape.
    builder.emit(abi::branch(&none));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        style + ITEM_STROKE_JOIN_BASE,
    ));
    builder.emit(abi::store_u32(
        records,
        abi::stack_pointer(),
        style + ITEM_STROKE_JOIN_COUNT,
    ));
    // target = mapped + cursor * 16; the cursor moves past the records' slots.
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        4,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[1]));
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], slots));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::move_immediate(
        residual_target,
        "Integer",
        &(VULKAN_EDGE_RESIDUAL_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(residual_target, residual_target, target));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[0],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[0]));
    // source = header + (slot1 - stops * 5 - dashes - records * 8) * 8: the dash tail
    // (plan-163-B) sits between the records and the stops.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 8));
    builder.emit(abi::float_convert_to_signed_x(index, abi::FP_SCRATCH[1]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[7], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    builder.emit(abi::shift_left_immediate(abi::SCRATCH[0], records, 3));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_DASH_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    // plan-163-C: and the path clip's edges, five floats each, after the dashes.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[7], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    builder.emit(abi::shift_left_immediate(index, index, 3));
    builder.emit(abi::add_registers(source, source, index));
    builder.emit(abi::move_immediate(index, "Integer", "0"));

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, slots));
    builder.emit(abi::branch_ge(&done));
    for word in 0..4usize {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, word * 8));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
        builder.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[0],
        ));
        builder.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
        builder.emit(abi::add_immediate(residual_target, residual_target, 4));
    }
    builder.emit(abi::add_immediate(source, source, 32));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&none));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in [ITEM_STROKE_JOIN_BASE, ITEM_STROKE_JOIN_COUNT] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            style + word,
        ));
    }
    builder.emit(abi::label(&done));
}
