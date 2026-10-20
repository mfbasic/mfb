//! plan-176-V: the per-item parameter block and the gradient stops upload. The shared
//! constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// Fill the 128-byte item block at `sp + off_item` from the geometry header whose
/// address is in `SCRATCH[0]`.
///
/// The Vulkan-flavoured twin of the Metal emitter in
/// `target/macos_aarch64/app/metal.rs`. The two emit into different IRs — one builds
/// a standalone `CodeFunction` with `Asm`, the other writes into an `abi_function`
/// body through `CodeBuilder` — but they write the *same* block, because the layout
/// and the header slots are one definition in `runtime/canvas/scene_layout.rs`. A change to
/// the contract therefore cannot move one backend without moving the other.
///
/// Positions narrow to 16.16 fixed point; colours cross as the whole 0–255 values
/// the header already stores, so nothing rounds a colour.
pub(super) fn emit_item_block(
    builder: &mut CodeBuilder,
    off_item: usize,
    off_width: usize,
    off_height: usize,
) {
    let header = abi::SCRATCH[0];
    let scale = abi::FP_SCRATCH[0];
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[1]));

    // The 16.16 fields no distance reads: the bounds and the gradient axis.
    for (item_offset, slots) in [
        (
            ITEM_OFFSET_QUAD,
            [
                HEADER_BOUNDS,
                HEADER_BOUNDS + 1,
                HEADER_BOUNDS + 2,
                HEADER_BOUNDS + 3,
            ],
        ),
        // plan-116-F: the gradient's axis, four consecutive header slots narrowing to
        // 16.16 exactly like the bounds and the arc caps.
        (
            ITEM_OFFSET_GRADIENT,
            [
                HEADER_GRADIENT_FROM_X,
                HEADER_GRADIENT_FROM_X + 1,
                HEADER_GRADIENT_FROM_X + 2,
                HEADER_GRADIENT_FROM_X + 3,
            ],
        ),
    ] {
        for (index, slot) in slots.into_iter().enumerate() {
            builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
            builder.emit(abi::float_multiply_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                scale,
            ));
            builder.emit(abi::float_round_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            builder.emit(abi::store_u32(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                off_item + item_offset + index * 4,
            ));
        }
    }
    // bug-1083: how much of that box is rim, 16.16 like the box, in the two words the
    // vertex stage reads it from under a group matrix (`ITEM_RIM_WORDS`).
    for (index, word) in crate::codegen::runtime::canvas::ITEM_RIM_WORDS
        .into_iter()
        .enumerate()
    {
        let slot = crate::codegen::runtime::canvas::HEADER_RIM + index;
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + word,
        ));
    }

    // bug-735: every field a distance reads -- the shape parameters, the radius and
    // stroke half-width, an arc's sweep endpoints, an ellipse's rotation and the clip --
    // as its 16.16 word and, in the block's residual `ivec4`s, its residual. The twin of
    // the loop in Metal's `emit_item_block`, from the same table.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[1],
    ));
    for (slot, word, residual) in ITEM_RESIDUAL_FIELDS {
        for instruction in
            emit_item_residual_field(header, slot, off_item + word, off_item + residual)
        {
            builder.emit(instruction);
        }
    }
    let positive = builder.label("vk_item_stroke_positive");
    let signed = builder.label("vk_item_stroke_done");
    for instruction in emit_item_stroke_sign(header, off_item, &positive, &signed) {
        builder.emit(instruction);
    }
    // bug-735: `arc.x` = 0 and an arc's sweep turn in `arc.y` (bug-1088), decided in f64.
    let set = builder.label("vk_item_reflex_set");
    let done = builder.label("vk_item_reflex_done");
    for instruction in emit_item_arc_flags(header, off_item, &set, &done) {
        builder.emit(instruction);
    }
    // plan-163-A: a polygon's fill rule over that word, which only an arc reads.
    let rule_done = builder.label("vk_item_rule_done");
    for instruction in emit_item_polygon_rule(header, off_item, &rule_done) {
        builder.emit(instruction);
    }

    // Both colours, as whole numbers.
    for (item_offset, first) in [
        (ITEM_OFFSET_FILL, HEADER_FILL_R),
        (ITEM_OFFSET_STROKE, HEADER_STROKE_R),
    ] {
        for channel in 0..4 {
            builder.emit(abi::load_double(
                abi::FP_SCRATCH[1],
                header,
                (first + channel) * 8,
            ));
            builder.emit(abi::float_convert_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            builder.emit(abi::store_u32(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                off_item + item_offset + channel * 4,
            ));
        }
    }

    // misc = { kind, radius, strokeHalf, edgeCount }: the two whole numbers here, the
    // two 16.16 ones with their residuals above.
    for (index, slot) in [(0usize, HEADER_KIND), (3, HEADER_AUX0)] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_MISC + index * 4,
        ));
    }

    for (index, parked) in [off_width, off_height].into_iter().enumerate() {
        builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_SURFACE + index * 4,
        ));
    }

    // The inverse transform (plan-116-C). The header already holds these as float32
    // BIT PATTERNS — `__canvas_float32Bits` narrowed them once, in MFBASIC, because
    // this assembler has no double→single convert — so the emitter's whole job is a
    // whole-number read and a 32-bit store. Seven slots: `ia..ity` then the flag.
    for (index, slot) in [
        HEADER_TRANSFORM_IA,
        HEADER_TRANSFORM_IB,
        HEADER_TRANSFORM_IC,
        HEADER_TRANSFORM_ID,
        HEADER_TRANSFORM_ITX,
        HEADER_TRANSFORM_ITY,
        HEADER_HAS_TRANSFORM,
    ]
    .into_iter()
    .enumerate()
    {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_TRANSFORM + index * 4,
        ));
    }

    // plan-163-D: a radial gradient's own inverse -- six float32 bit patterns and the
    // flag, header slots 57-63 -- word for word into `gradInv0`/`gradInv1`, the
    // transform's layout; and its focal offset and radius narrowed to 16.16 (`gradFocal`).
    for index in 0..7 {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            header,
            (HEADER_GRADIENT_INV + index) * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_GRADIENT_INV + index * 4,
        ));
    }
    debug_assert_eq!(HEADER_GRADIENT_INV + 6, HEADER_GRADIENT_HAS_INV);
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[1],
    ));
    for index in 0..3 {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            header,
            (HEADER_GRADIENT_FOCAL_X + index) * 8,
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_GRADIENT_FOCAL + index * 4,
        ));
    }

    // The blend tag, a whole 0..3 beside the surface size (plan-116-B). `Normal` is 0,
    // so an item that never set `Paint.blend` writes the value the pipeline it selects
    // has always had.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_BLEND * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_SURFACE + ITEM_SURFACE_BLEND,
    ));

    // The cap tag, a whole 0 or 1 in the per-kind block's last free word (plan-116-D).
    // Copied for every kind rather than under a `Line`/`Arc` branch: it is one store,
    // and the branch would cost more than it saves while giving the word two meanings
    // depending on how the item got here.
    // plan-116-F: the gradient's kind, a whole 0 or 1 in the block's last spare word.
    // plan-163-D: packed with the spread and the stroke apply, `kind + 2 * spread + 8 *
    // apply` (`ITEM_SURFACE_GRADIENT_KIND`), every term a small whole number.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_KIND * 8,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[2],
        header,
        HEADER_GRADIENT_SPREAD * 8,
    ));
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[2],
        header,
        HEADER_GRADIENT_APPLY * 8,
    ));
    for _ in 0..3 {
        builder.emit(abi::float_add_d(
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
        ));
    }
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_SURFACE + ITEM_SURFACE_GRADIENT_KIND,
    ));

    builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, HEADER_CAP * 8));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ARC + ITEM_ARC_CAP,
    ));
}

/// Append this item's polygon edges to the frame's edge buffer and record where they
/// landed.
///
/// **Why an offset rather than a rebind.** A Vulkan command buffer is recorded now
/// and executed later, all at once. There is one edge buffer for the frame, so
/// rewriting it per item — or rebinding the same buffer per item — would give every
/// polygon whatever the *last* one wrote. Each polygon therefore takes a slice and
/// carries its start index in its own item block, which is per-item. Metal has the
/// opposite property and needs none of this: `setFragmentBytes:` copies the bytes
/// into the command buffer at record time.
///
/// The geometry cache stores each edge as `x0, y0, dx, dy, invLenSq` doubles; the
/// shader wants the two endpoints in 16.16, and recomputes the rest. `invLenSq` is
/// the one quantity fixed point represents badly — a 100-px edge gives 1e-4, which
/// is 6 in 16.16 — and a GPU has the divide for free, so dropping it from the
/// payload is both smaller and more accurate.
/// Copy this item's gradient stops into the frame buffer's third region, and record
/// where they landed in the item block.
///
/// The twin of `emit_edge_upload`, deliberately the same shape: one buffer, three
/// regions, one binding, with a per-item base index rather than a rebind. A gradient's
/// stops are variable-length per item for exactly the reason a polygon's edges are, so
/// the machinery that already solved that is the machinery to reuse.
///
/// The stops sit at the END of the geometry record — `slot1 − count * 5` — because
/// `__canvas_tailFor` appends them after whatever other tail the kind has. Computing
/// the base from the record's own length is what lets this work for a gradient-filled
/// polygon, whose tail is edges *then* stops, without the emitter knowing an edge count.
///
/// The offset is 16.16 like every other coordinate; the four channels are whole 0..255
/// bytes, which is how the item block already carries `fill` and `stroke`.
pub(super) fn emit_gradient_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_grad_cursor: usize,
) {
    let head = builder.label("vk_grad_head");
    let done = builder.label("vk_grad_done");
    let empty = builder.label("vk_grad_empty");
    let copy = builder.label("vk_grad_copy");

    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let scale = abi::FP_SCRATCH[0];

    // Fewer than two stops is not a gradient, and the header already says so — the
    // count slot is written as 0 in that case, so one compare covers "no gradient",
    // "one stop" and "a kind with no interior".
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_header,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[6],
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(count, "2"));
    builder.emit(abi::branch_lt(&empty));

    // Does this frame still have room? The predicate already declined a scene whose
    // stops do not fit, so this is unreachable — kept for the reason the edge one is:
    // the alternative to declining is writing past the buffer, and truncating a ramp
    // would draw a *different picture*.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_grad_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], count));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_GRADIENT_STOPS.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&empty));

    builder.emit(abi::label(&copy));
    // The block carries the count and the base, in stops.
    builder.emit(abi::store_u32(
        count,
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_BASE,
    ));

    // target = mapped + (GRADIENT_BASE_WORDS + cursor * 5) * 4
    builder.emit(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &GRADIENT_STOP_WORDS.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        VULKAN_GRADIENT_BASE_WORDS,
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

    // The cursor moves on now, while the pre-advance value is already in the block.
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_grad_cursor,
    ));

    // source = header + (slot1 - count * 5) * 8, the record's own stop base.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], abi::SCRATCH[6], 8));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[8],
        count,
        abi::SCRATCH[7],
    ));
    builder.emit(abi::subtract_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        3,
    ));
    builder.emit(abi::add_registers(source, abi::SCRATCH[6], abi::SCRATCH[1]));

    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[0]));
    builder.emit(abi::move_immediate(index, "Integer", "0"));

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&done));
    // The offset, 16.16.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 0));
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
    // The four channels, whole 0..255 — the form the block already uses for fill.
    for channel in 1..=4usize {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, channel * 8));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
    }
    builder.emit(abi::add_immediate(source, source, GRADIENT_STOP_WORDS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_BASE,
    ));

    builder.emit(abi::label(&done));
}
