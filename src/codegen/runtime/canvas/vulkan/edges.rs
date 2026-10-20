//! plan-176-V: the edge and clip-path uploads. The shared constants, layouts and imports live
//! in `runtime/canvas/vulkan/mod.rs`.

use super::*;

pub(super) fn emit_edge_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_edge_cursor: usize,
    bands: &BandSlots,
) {
    let head = builder.label("vk_edge_head");
    let done = builder.label("vk_edge_done");
    let band = builder.label("vk_edge_band");
    let empty = builder.label("vk_edge_empty");
    let copy = builder.label("vk_edge_copy");
    let polyline = builder.label("vk_edge_polyline");
    let has_pieces = builder.label("vk_edge_pieces");
    let advanced = builder.label("vk_edge_advanced");
    let twice = builder.label("vk_edge_twice");
    let chords = builder.label("vk_edge_chords");
    let controls_init = builder.label("vk_edge_controls_init");
    let controls = builder.label("vk_edge_controls");

    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];

    // Only a polygon has edges. Every other kind leaves `misc.w` holding whatever
    // `HEADER_AUX0` meant for it — an arc's start angle — which is harmless because
    // the shader reads `misc.w` only in the polygon arm, but zeroing it keeps the
    // block meaning one thing at a time.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    let has_edges = builder.label("vk_edge_has");
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYGON));
    builder.emit(abi::branch_eq(&has_edges));
    // plan-154-C: a PolyLine's segments are edges in the polygon layout, one per segment
    // and without a wrap edge, so they take the same upload and band index -- the twin of
    // Metal's `emit_edge_buffer` (plan-154-B).
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYLINE));
    builder.emit(abi::branch_eq(&polyline));
    // plan-164-C: a curve shape's pieces take the same region, TWO slots a piece, and
    // the same band index -- the twin of Metal's `emit_edge_buffer`.
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYCURVE));
    builder.emit(abi::branch_eq(&has_pieces));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        GEO_KIND_CURVEPOLYGON,
    ));
    builder.emit(abi::branch_ne(&empty));
    builder.emit(abi::label(&has_pieces));
    builder.emit(abi::load_u32(
        count,
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], count));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[1], count));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&empty));
    builder.emit(abi::label(&polyline));
    // Its first and last live segments, as whole numbers over the 16.16 gradient axis
    // `emit_item_block` wrote: 16.16 holds a segment index only up to 32,767.
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_header,
    ));
    for (index, slot) in [(0usize, HEADER_POLYLINE_FIRST), (1, HEADER_POLYLINE_LAST)] {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[1],
            slot * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_GRADIENT + index * 4,
        ));
    }
    builder.emit(abi::label(&has_edges));

    builder.emit(abi::load_u32(
        count,
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], count));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    // Unreachable: the pass walk (`__canvas_passNext`, bug-765) ends a pass before its
    // polygon edges pass the region, and a polygon past it alone sends the whole frame
    // to the software rasteriser.
    // Kept because the alternative to declining is writing past the buffer, and
    // truncating the polygon would draw a *different shape* — the same reason Metal's
    // `emit_edge_buffer` refuses rather than clamps.
    builder.emit(abi::branch(&empty));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    // target = mapped + cursor * 16
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
    // The cursor moves on now, while the pre-advance value is still the one written
    // into the item block above -- by one slot an edge, and by two a curve piece.
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_POLYCURVE));
    builder.emit(abi::branch_eq(&twice));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        GEO_KIND_CURVEPOLYGON,
    ));
    builder.emit(abi::branch_ne(&advanced));
    builder.emit(abi::label(&twice));
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::label(&advanced));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));

    // bug-687: each word's residual goes to the same edge's slot in the residual region,
    // a fixed distance past its 16.16 word (`VULKAN_EDGE_RESIDUAL_BASE_WORDS`) — Metal's
    // `emit_edge_buffer` twin.
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
    builder.emit(abi::load_u64(source, abi::stack_pointer(), off_header));
    builder.emit(abi::add_immediate(source, source, HEADER_SLOTS * 8));
    builder.emit(abi::move_immediate(index, "Integer", "0"));

    // plan-164-C: a curve shape's pieces (`x0, y0, x1, y1, x2, y2, w1` each) go in as
    // two runs of `count` slots: each piece's chord `x0, y0, x2, y2` -- an edge to the
    // band index, a piece's two ends being its whole y-range -- then each piece's
    // control point and weight, `x1, y1, bits(w1), 0`. Metal's `emit_edge_buffer`, line for
    // line; the layout is at `CURVE_PIECE_SLOTS`.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYCURVE));
    builder.emit(abi::branch_eq(&chords));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        GEO_KIND_CURVEPOLYGON,
    ));
    builder.emit(abi::branch_ne(&head));

    // One double of the record as its 16.16 word and its residual, each cursor advanced.
    let word = |builder: &mut CodeBuilder, slot: usize| {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
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
    };
    builder.emit(abi::label(&chords));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&controls_init));
    for slot in [0usize, 1, 4, 5] {
        word(builder, slot);
    }
    builder.emit(abi::add_immediate(source, source, CURVE_PIECE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&chords));
    builder.emit(abi::label(&controls_init));
    builder.emit(abi::load_u64(source, abi::stack_pointer(), off_header));
    builder.emit(abi::add_immediate(source, source, HEADER_SLOTS * 8));
    builder.emit(abi::move_immediate(index, "Integer", "0"));
    builder.emit(abi::label(&controls));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&band));
    for slot in [2usize, 3] {
        word(builder, slot);
    }
    // The weight as its fp32 BITS, not a 16.16 word: the shader's arithmetic on it is
    // single precision either way, and a weight has no 32767 to stay under.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 6 * 8));
    builder.emit(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 4));
    builder.emit(abi::add_immediate(target, target, 8));
    builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
    builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 4));
    builder.emit(abi::add_immediate(residual_target, residual_target, 8));
    builder.emit(abi::add_immediate(source, source, CURVE_PIECE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&controls));

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&band));
    // out[0..1] = (x0, y0); out[2..3] = (x0 + dx, y0 + dy), each as its 16.16 word and,
    // in the residual region, `round((v * 65536 - word) * 2^31)` (bug-687,
    // `EDGE_RESIDUAL_SCALE`). `v * 65536 - word` is exact: the two are within half a
    // step of each other.
    for (slot, delta) in [(0usize, None), (1, None), (0, Some(2usize)), (1, Some(3))] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
        if let Some(delta) = delta {
            builder.emit(abi::load_double(abi::FP_SCRATCH[2], source, delta * 8));
            builder.emit(abi::float_add_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[2],
            ));
        }
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
    builder.emit(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

    // bug-688: the polygon's edges are in the region; index them by band.
    builder.emit(abi::label(&band));
    emit_band_index(builder, bands, ITEM_BANDS);
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));

    builder.emit(abi::label(&done));
}

/// plan-163-B: the item's join records (`helper_joins.rs`) into the frame's edge region
/// after its own edges, two edge slots per record -- each record's four points as eight
/// 16.16 words, with their residuals at the residual region's same offsets, exactly as an
/// edge's endpoints travel. `strokeStyle.x` is the first slot and `strokeStyle.y` the
/// record count; both are 0 for an item with none, and so are the dash words. The
/// records sit right before the gradient stops, so they are found from the record's end:
/// `header + (slot1 - stops * 5 - records * 8) * 8`. Metal's `emit_join_buffer` twin.
/// plan-163-C: the clip's item-block words -- `clipPath` = (first clip edge, clip edge
/// count, mode | rule << 2, a picture's transparency step T) and `clipBand`'s band count
/// 0 (the shader walks every clip edge) -- and a path clip's edges, copied from the
/// record's tail (`slot1 − stops·5 − edges·5`, right before the stops) into the frame's
/// edge region after the item's own edges and join records, as 16.16 words with their
/// residuals, exactly as a polygon's edges are (`emit_edge_upload`). The frame-sum edge cap
/// counts them, so the over-cap arm is unreachable; it leaves the edge count 0, which the
/// shader reads as a clip enclosing nothing -- hidden, never drawn unclipped.
pub(super) fn emit_clip_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_edge_cursor: usize,
    bands: &BandSlots,
) {
    let head = builder.label("vk_clip_head");
    let done = builder.label("vk_clip_done");
    let copy = builder.label("vk_clip_copy");
    let banded = builder.label("vk_clip_banded");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[6];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];
    let clip = off_item + ITEM_OFFSET_CLIP_PATH;
    let band = off_item + ITEM_OFFSET_CLIP_BAND;

    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    for word in [0usize, 4, 8, 12] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            band + word,
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            clip + word,
        ));
    }
    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    // mode | rule << 2, and T.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_MODE * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_RULE * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        2,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[5],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_MODE,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_TRANSPARENCY * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_PICTURE_T,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[5], "2"));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(count, "0"));
    builder.emit(abi::branch_le(&done));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[7], abi::SCRATCH[5], count));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[8],
        "Integer",
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[7], abi::SCRATCH[8]));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_BASE,
    ));
    builder.emit(abi::store_u32(
        count,
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_COUNT,
    ));
    // target = mapped + base * 16; the residuals a fixed distance past it.
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[7],
        abi::SCRATCH[5],
        4,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[7]));
    builder.emit(abi::move_immediate(
        residual_target,
        "Integer",
        &(VULKAN_EDGE_RESIDUAL_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(residual_target, residual_target, target));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[5],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[5]));
    // source = header + (slot1 - stops * 5 - count * 5) * 8.
    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
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
    builder.emit(abi::add_registers(abi::SCRATCH[7], abi::SCRATCH[7], count));
    builder.emit(abi::move_immediate(abi::SCRATCH[8], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    builder.emit(abi::shift_left_immediate(source, source, 3));
    builder.emit(abi::add_registers(source, header, source));
    builder.emit(abi::move_immediate(index, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&banded));
    for (slot, delta) in [(0usize, None), (1, None), (0, Some(2usize)), (1, Some(3))] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
        if let Some(delta) = delta {
            builder.emit(abi::load_double(abi::FP_SCRATCH[2], source, delta * 8));
            builder.emit(abi::float_add_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[2],
            ));
        }
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[5], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
        builder.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[5],
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
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[5], residual_target, 0));
        builder.emit(abi::add_immediate(residual_target, residual_target, 4));
    }
    builder.emit(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));
    // The clip's edges are in the region; index them by band.
    builder.emit(abi::label(&banded));
    emit_band_index(builder, bands, CLIP_BANDS);
    builder.emit(abi::label(&done));
}
