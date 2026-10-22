//! plan-176-V: the polygon band index — `BandSlots`, the item and clip band sets and their
//! emitters. The shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// The stack slots `emit_band_index` works in (bug-688): the frame's band-region cursor
/// and one polygon's working state — its first and past-the-last edge in the shared
/// buffer, its lowest edge y (16.16), the band reach `2r` (16.16), the band height
/// (16.16), the band count, and the table's address. The builder's arithmetic is 16.16
/// integers; the edges and the header it writes are float32 (`emit_f32_word_to_fixed`,
/// `emit_fixed_to_f32_bits`).
pub(super) struct BandSlots {
    state: usize,
    item: usize,
    header: usize,
    pub(super) cursor: usize,
    edges: usize,
    end: usize,
    ymin: usize,
    reach: usize,
    height: usize,
    count: usize,
    table: usize,
}

impl BandSlots {
    pub(super) fn allocate(
        builder: &mut CodeBuilder,
        state: usize,
        item: usize,
        header: usize,
    ) -> Self {
        let mut slot = |name: &str| builder.allocate_stack_object(name, 8);
        BandSlots {
            state,
            item,
            header,
            cursor: slot("vk_band_cursor"),
            edges: slot("vk_band_edges"),
            end: slot("vk_band_end"),
            ymin: slot("vk_band_ymin"),
            reach: slot("vk_band_reach"),
            height: slot("vk_band_height"),
            count: slot("vk_band_count"),
            table: slot("vk_band_table"),
        }
    }
}

/// Which edge set a band index is built for: the item's own edges, its header in
/// `arcCaps`, or (plan-163-C) a path clip's, its header in `clipBand` -- Metal's
/// `BandSet`, with offsets into the block rather than into Metal's frame.
#[derive(Clone, Copy)]
pub(super) struct BandSet {
    count: usize,
    base: usize,
    header: usize,
    /// bug-1085: the header's norm word (`ITEM_BAND_NORM`, `ITEM_CLIP_BAND_NORM`).
    norm: usize,
    clip: bool,
}

pub(super) const ITEM_BANDS: BandSet = BandSet {
    count: ITEM_OFFSET_MISC + 12,
    base: ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    header: ITEM_OFFSET_ARC_CAPS,
    norm: ITEM_BAND_NORM,
    clip: false,
};

pub(super) const CLIP_BANDS: BandSet = BandSet {
    count: ITEM_OFFSET_CLIP_PATH + ITEM_CLIP_PATH_COUNT,
    base: ITEM_OFFSET_CLIP_PATH + ITEM_CLIP_PATH_BASE,
    header: ITEM_OFFSET_CLIP_BAND,
    norm: ITEM_CLIP_BAND_NORM,
    clip: true,
};

/// Build this polygon's band index in the shared buffer's band region and write its
/// header into the item block's `arcCaps` (bug-688; layout at `VULKAN_BAND_BASE_WORDS`)
/// -- or a path clip's into `clipBand` (`set`).
///
/// The Vulkan port of Metal's `emit_band_index` (bug-686 section G), the same algorithm
/// step for step, so a polygon's bands — and therefore its pixels — are the same on both
/// backends. It is a port rather than one shared builder because Metal's is written
/// against the macOS `Asm` and a hand-numbered stack frame, and sharing would re-emit
/// Metal's instructions; bug-688 keeps Metal unchanged.
///
/// Runs after `emit_edge_upload` has converted the polygon's edges, and works on those
/// float32 words — exactly what the shader reads — taken to 16.16 integers (within 2^-17
/// of a unit, inside the guard below), so the band arithmetic is integer.
///
/// ## Why the result is bit-identical to the full loop
///
/// An edge is put in every band its `[ylo − r, yhi + r]` meets, so a band holds (a)
/// every edge whose y-span contains any row of the band — the only edges the even-odd
/// crossing test counts, so `inside` is exact — and (b) every edge within `r` of any
/// point of the band — so `best` is exact whenever the true distance is at most `r`,
/// and otherwise both loops' `best` exceed `r`, where coverage has clamped:
///
/// * **Untransformed**, a fill clamps once `|d| ≥ 0.5` and a stroke once
///   `|d| − half ≥ 0.5`, so `r = half + 0.5`, with `half = max(strokeHalf, 0)`.
/// * **Transformed**, the shader evaluates five shape-space points against the centre's
///   band; with `F = ‖M‖_F` (M the inverse matrix) `r = half + 0.5 + 2F` covers every
///   neighbour and every scale the shader can divide by — the argument is written out at
///   Metal's `emit_band_index`.
/// * **Under a group's matrix** (bug-1085) the argument needs the COMPOSED norm `N`,
///   which no builder can know (a block is drawn under every group matrix that names
///   it). The reach is built for `F_built = max(F, √2)` (the identity's norm at least, so
///   no group, an identity, a rotation or a magnifying group keeps the band), the header
///   records it (`ITEM_BAND_NORM`, `ITEM_CLIP_BAND_NORM`), and the shader takes the band
///   only when `1.5·N <= 2·F_built`, walking every edge otherwise.
///
/// Plus a **1-unit guard** in shape space, for the float error of the shader's band
/// lookup against these integer bands, and of the float32 header against them.
///
/// ## Sizing
///
/// Bands are `√2·F_built` tall (2 px untransformed), and taller when the span needs
/// more than `min(edges, BAND_MAX)` bands. If the list would pass `BAND_LIST_PER_EDGE`
/// entries per edge the height doubles until it fits; at one band the list is every edge
/// once. So one polygon never needs more than `BAND_WORDS_PER_EDGE` words per edge, which
/// the region is sized for; the region-full branch is unreachable and kept because the
/// alternative is a write past the buffer. It, a polygon under `BAND_MIN_EDGES`, and a
/// transform whose matrix is not finite write band count 0: the shader loops over every
/// edge. Never a decline, never a truncation.
///
/// ## Differences from the AArch64-only original
///
/// * **Every float-to-integer result is checked for a negative value.** AArch64's
///   `fcvtps` saturates an infinity to `i64::MAX`, which the `2^30` bound catches; x86-64's
///   `cvtsd2si` returns `i64::MIN` for anything out of range, which that bound does not.
/// * No calls; `SCRATCH[0..=8]` only, and no `compare_immediate` against a constant wider
///   than 12 bits (on AArch64 its encoding borrows x16/x17, which are `SCRATCH[7]`/`[8]`).
pub(super) fn emit_band_index(builder: &mut CodeBuilder, at: &BandSlots, set: BandSet) {
    let none = builder.label("vk_band_none");
    let done = builder.label("vk_band_done");
    let s = abi::SCRATCH;
    let f = abi::FP_SCRATCH;
    let block = |field: usize| at.item + set.header + field;
    let sp = abi::stack_pointer;

    // --- which polygons: E >= BAND_MIN_EDGES --------------------------------------
    builder.emit(abi::load_u32(s[2], sp(), at.item + set.count));
    builder.emit(abi::move_immediate(
        s[3],
        "Integer",
        &BAND_MIN_EDGES.to_string(),
    ));
    builder.emit(abi::compare_registers(s[2], s[3]));
    builder.emit(abi::branch_lt(&none));

    // --- the edges: P = mapped + base*16, END = P + E*16 ------------------------------
    builder.emit(abi::load_u32(s[0], sp(), at.item + set.base));
    builder.emit(abi::shift_left_immediate(s[0], s[0], 4));
    emit_state_load(builder, at.state, GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED, s[1]);
    builder.emit(abi::add_registers(s[0], s[0], s[1]));
    builder.emit(abi::store_u64(s[0], sp(), at.edges));
    builder.emit(abi::shift_left_immediate(s[1], s[2], 4));
    builder.emit(abi::add_registers(s[1], s[0], s[1]));
    builder.emit(abi::store_u64(s[1], sp(), at.end));

    // --- ymin (s2) and ymax (s3) over both endpoints of every edge ----------------
    let measure_head = builder.label("vk_band_measure_head");
    let measure_done = builder.label("vk_band_measure_done");
    for instruction in emit_f32_word_to_fixed(s[2], s[0], 4, s[4], f[0], f[7]) {
        builder.emit(instruction);
    }
    builder.emit(abi::move_register(s[3], s[2]));
    builder.emit(abi::label(&measure_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&measure_done));
    for offset in [4usize, 12] {
        let not_lower = builder.label("vk_band_not_lower");
        let not_higher = builder.label("vk_band_not_higher");
        for instruction in emit_f32_word_to_fixed(s[4], s[0], offset, s[5], f[0], f[7]) {
            builder.emit(instruction);
        }
        builder.emit(abi::compare_registers(s[4], s[2]));
        builder.emit(abi::branch_ge(&not_lower));
        builder.emit(abi::move_register(s[2], s[4]));
        builder.emit(abi::label(&not_lower));
        builder.emit(abi::compare_registers(s[4], s[3]));
        builder.emit(abi::branch_le(&not_higher));
        builder.emit(abi::move_register(s[3], s[4]));
        builder.emit(abi::label(&not_higher));
    }
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::branch(&measure_head));
    builder.emit(abi::label(&measure_done));
    builder.emit(abi::store_u64(s[2], sp(), at.ymin));
    // s3 = the edges' own y extent; s2 stays ymin through the reach computation.
    builder.emit(abi::subtract_registers(s[3], s[3], s[2]));

    // --- the reach r (f1) and the minimum band height (f4), in shape units --------
    builder.emit(abi::load_u64(s[0], sp(), at.header));
    builder.emit(abi::load_double(f[1], s[0], HEADER_STROKE_HALF * 8));
    // plan-163-C: a clip's coverage clamps at `|d| >= 0.5` -- no stroke reaches past it --
    // so its half-width is 0 (and its own cap and join reach do not apply).
    let stroke_reach_done = builder.label("vk_band_stroke_reach_done");
    if set.clip {
        builder.emit(abi::move_immediate(s[4], "Integer", "0"));
        builder.emit(abi::signed_convert_to_float_d(f[1], s[4]));
        builder.emit(abi::branch(&stroke_reach_done));
    }
    // plan-154-C: a PolyLine keeps its half-width in the RADIUS slot (it is
    // stroke-as-fill, slot 7 is -1); a polygon never writes that slot, so its reach is
    // unchanged. Metal's `emit_band_index` takes the same maximum.
    builder.emit(abi::load_double(f[2], s[0], HEADER_RADIUS * 8));
    builder.emit(abi::float_max_d(f[1], f[1], f[2]));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::signed_convert_to_float_d(f[2], s[4]));
    // `fmaxnm`: a NaN half-width reads as 0 rather than poisoning the reach.
    builder.emit(abi::float_max_d(f[1], f[1], f[2]));
    // plan-163-B: a Square cap reaches `half * sqrt(2)` past its endpoint.
    let square_reach = builder.label("vk_band_square_reach");
    for instruction in emit_square_cap_reach(s[0], f[1], s[4], f[2], &square_reach) {
        builder.emit(instruction);
    }
    // plan-163-B: and to a join record's reach (0 for an item with none), so a miter's
    // point in a row's band is reached through the edge it sits at the start of.
    builder.emit(abi::load_double(f[2], s[0], HEADER_JOIN_REACH * 8));
    builder.emit(abi::float_max_d(f[1], f[1], f[2]));
    builder.emit(abi::label(&stroke_reach_done));
    builder.emit(abi::move_immediate(s[4], "Integer", "3"));
    builder.emit(abi::signed_convert_to_float_d(f[2], s[4]));
    builder.emit(abi::move_immediate(s[4], "Integer", "2"));
    builder.emit(abi::signed_convert_to_float_d(f[3], s[4]));
    builder.emit(abi::float_divide_d(f[2], f[2], f[3]));
    // r = half + 0.5 (the clamp) + 1 (the guard)
    builder.emit(abi::float_add_d(f[1], f[1], f[2]));
    builder.emit(abi::float_move_d_from_d(f[4], f[3]));
    // F (f6) is 0 untransformed, before the floor below (bug-1085).
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::signed_convert_to_float_d(f[6], s[4]));
    let untransformed = builder.label("vk_band_untransformed");
    builder.emit(abi::load_double(f[5], s[0], HEADER_HAS_TRANSFORM * 8));
    builder.emit(abi::float_convert_to_signed_x(s[4], f[5]));
    builder.emit(abi::compare_immediate(s[4], "0"));
    builder.emit(abi::branch_eq(&untransformed));
    // F = ||M||_F from the four matrix terms, which the header carries as float32 BIT
    // PATTERNS (`__canvas_float32Bits`). The assemblers have no single->double convert,
    // so the bits are rebuilt as a double by hand: exponent rebiased by 1023 - 127 =
    // 896, mantissa shifted up 52 - 23 = 29. The sign is dropped (the term is squared);
    // a zero or subnormal term counts as 0, which the 1-unit guard dwarfs; an infinite
    // or NaN one gives the polygon no bands.
    for slot in [
        HEADER_TRANSFORM_IA,
        HEADER_TRANSFORM_IB,
        HEADER_TRANSFORM_IC,
        HEADER_TRANSFORM_ID,
    ] {
        let zero_term = builder.label("vk_band_zero_term");
        builder.emit(abi::load_double(f[7], s[0], slot * 8));
        builder.emit(abi::float_convert_to_signed_x(s[4], f[7]));
        builder.emit(abi::shift_right_immediate(s[5], s[4], 23));
        builder.emit(abi::move_immediate(s[6], "Integer", "255"));
        builder.emit(abi::and_registers(s[5], s[5], s[6]));
        builder.emit(abi::compare_registers(s[5], s[6]));
        builder.emit(abi::branch_eq(&none));
        builder.emit(abi::compare_immediate(s[5], "0"));
        builder.emit(abi::branch_eq(&zero_term));
        builder.emit(abi::add_immediate(s[5], s[5], 896));
        builder.emit(abi::shift_left_immediate(s[5], s[5], 52));
        builder.emit(abi::move_immediate(s[6], "Integer", "8388607"));
        builder.emit(abi::and_registers(s[7], s[4], s[6]));
        builder.emit(abi::shift_left_immediate(s[7], s[7], 29));
        builder.emit(abi::or_registers(s[5], s[5], s[7]));
        builder.emit(abi::float_move_d_from_x(f[7], s[5]));
        builder.emit(abi::float_multiply_d(f[7], f[7], f[7]));
        builder.emit(abi::float_add_d(f[6], f[6], f[7]));
        builder.emit(abi::label(&zero_term));
    }
    builder.emit(abi::float_sqrt_d(f[6], f[6]));
    builder.emit(abi::label(&untransformed));
    // bug-1085: the reach is built for F_built = max(F, sqrt 2), the identity's norm at
    // least, so a pixel drawn under no group or under an identity, rotation or magnifying
    // group keeps the band (`0.75 N <= F_built`) and only a shrinking group walks every
    // edge. r += 2 F_built; the minimum height is sqrt(2) F_built, about two surface
    // pixels (exactly 2 untransformed, the height it always had).
    builder.emit(abi::float_sqrt_d(f[7], f[3]));
    builder.emit(abi::float_max_d(f[6], f[6], f[7]));
    builder.emit(abi::float_add_d(f[5], f[6], f[6]));
    builder.emit(abi::float_add_d(f[1], f[1], f[5]));
    builder.emit(abi::float_multiply_d(f[4], f[6], f[7]));

    // --- to 16.16, rounding UP: s4 = r, s5 = the minimum height ---------------------
    builder.emit(abi::move_immediate(s[4], "Integer", FIXED_POINT_SCALE));
    builder.emit(abi::signed_convert_to_float_d(f[7], s[4]));
    // bug-1085: and F_built, as float32 bits, into the header's norm word -- what the
    // shader holds a pixel's composed norm to before it takes the band. A reach past the
    // bound below sends the builder to `none`, which writes 0 over it.
    builder.emit(abi::float_move_d_from_d(f[0], f[6]));
    for instruction in emit_f32_bits(s[6], f[0]) {
        builder.emit(instruction);
    }
    builder.emit(abi::store_u32(s[6], sp(), at.item + set.norm));
    builder.emit(abi::float_multiply_d(f[1], f[1], f[7]));
    builder.emit(abi::float_ceil_to_signed_x(s[4], f[1]));
    builder.emit(abi::float_multiply_d(f[4], f[4], f[7]));
    builder.emit(abi::float_ceil_to_signed_x(s[5], f[4]));
    // A reach or height past 2^30 (16,384 units) is a degenerate transform. AArch64
    // saturates an out-of-range convert to i64::MAX, which the bound catches; x86-64
    // answers i64::MIN, which the sign test catches.
    builder.emit(abi::compare_immediate(s[4], "0"));
    builder.emit(abi::branch_lt(&none));
    builder.emit(abi::compare_immediate(s[5], "0"));
    builder.emit(abi::branch_lt(&none));
    builder.emit(abi::move_immediate(
        s[6],
        "Integer",
        &(1u64 << 30).to_string(),
    ));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_gt(&none));
    builder.emit(abi::compare_registers(s[5], s[6]));
    builder.emit(abi::branch_gt(&none));
    let height_positive = builder.label("vk_band_height_positive");
    builder.emit(abi::compare_immediate(s[5], "1"));
    builder.emit(abi::branch_ge(&height_positive));
    builder.emit(abi::move_immediate(s[5], "Integer", "1"));
    builder.emit(abi::label(&height_positive));

    // --- span = extent + 2r (s3); top = ymin - r, and the bottom, are held to int32,
    // the range the builder's band arithmetic was written for ------------------------
    builder.emit(abi::shift_left_immediate(s[4], s[4], 1));
    builder.emit(abi::store_u64(s[4], sp(), at.reach));
    builder.emit(abi::add_registers(s[3], s[3], s[4]));
    builder.emit(abi::shift_right_immediate(s[6], s[4], 1));
    builder.emit(abi::subtract_registers(s[7], s[2], s[6]));
    // The encoder parses an immediate as u64, so i32::MIN goes in as its 64-bit two's
    // complement.
    builder.emit(abi::move_immediate(
        s[8],
        "Integer",
        &(i64::from(i32::MIN) as u64).to_string(),
    ));
    builder.emit(abi::compare_registers(s[7], s[8]));
    builder.emit(abi::branch_lt(&none));
    builder.emit(abi::add_registers(s[8], s[7], s[3]));
    builder.emit(abi::move_immediate(s[6], "Integer", &i32::MAX.to_string()));
    builder.emit(abi::compare_registers(s[8], s[6]));
    builder.emit(abi::branch_gt(&none));

    // --- height = max(minimum, ceil(span / (min(E, BAND_MAX) - 1))) -----------------
    // At most `min(E, BAND_MAX)` bands: span / height <= that - 1.
    let bands_capped = builder.label("vk_band_bands_capped");
    builder.emit(abi::load_u32(s[6], sp(), at.item + set.count));
    builder.emit(abi::move_immediate(s[7], "Integer", &BAND_MAX.to_string()));
    builder.emit(abi::compare_registers(s[6], s[7]));
    builder.emit(abi::branch_le(&bands_capped));
    builder.emit(abi::move_register(s[6], s[7]));
    builder.emit(abi::label(&bands_capped));
    const _: () = assert!(BAND_MIN_EDGES >= 2 && BAND_MAX >= 2);
    builder.emit(abi::subtract_immediate(s[6], s[6], 1));
    builder.emit(abi::add_registers(s[7], s[3], s[6]));
    builder.emit(abi::subtract_immediate(s[7], s[7], 1));
    builder.emit(abi::unsigned_divide_registers(s[7], s[7], s[6]));
    let height_chosen = builder.label("vk_band_height_chosen");
    builder.emit(abi::compare_registers(s[7], s[5]));
    builder.emit(abi::branch_ge(&height_chosen));
    builder.emit(abi::move_register(s[7], s[5]));
    builder.emit(abi::label(&height_chosen));
    builder.emit(abi::store_u64(s[7], sp(), at.height));

    // --- size the list; double the height until it is within its budget -----------
    // s3 (span) is live through this loop.
    let fit_head = builder.label("vk_band_fit_head");
    let size_head = builder.label("vk_band_size_head");
    let size_done = builder.label("vk_band_size_done");
    let fits = builder.label("vk_band_fits");
    builder.emit(abi::label(&fit_head));
    builder.emit(abi::load_u64(s[4], sp(), at.height));
    builder.emit(abi::unsigned_divide_registers(s[6], s[3], s[4]));
    builder.emit(abi::add_immediate(s[6], s[6], 1));
    builder.emit(abi::store_u64(s[6], sp(), at.count));
    builder.emit(abi::move_immediate(s[2], "Integer", "0"));
    builder.emit(abi::load_u64(s[0], sp(), at.edges));
    builder.emit(abi::load_u64(s[1], sp(), at.end));
    builder.emit(abi::label(&size_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&size_done));
    emit_band_range(builder, at);
    builder.emit(abi::subtract_registers(s[5], s[5], s[4]));
    builder.emit(abi::add_immediate(s[5], s[5], 1));
    builder.emit(abi::add_registers(s[2], s[2], s[5]));
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::branch(&size_head));
    builder.emit(abi::label(&size_done));
    builder.emit(abi::load_u32(s[4], sp(), at.item + set.count));
    builder.emit(abi::move_immediate(
        s[5],
        "Integer",
        &BAND_LIST_PER_EDGE.to_string(),
    ));
    builder.emit(abi::multiply_registers(s[4], s[4], s[5]));
    builder.emit(abi::compare_registers(s[2], s[4]));
    builder.emit(abi::branch_le(&fits));
    builder.emit(abi::load_u64(s[4], sp(), at.height));
    builder.emit(abi::shift_left_immediate(s[4], s[4], 1));
    builder.emit(abi::store_u64(s[4], sp(), at.height));
    builder.emit(abi::branch(&fit_head));
    builder.emit(abi::label(&fits));

    // --- claim 2*bands + list words of the region ---------------------------------
    builder.emit(abi::load_u64(s[6], sp(), at.count));
    builder.emit(abi::shift_left_immediate(s[4], s[6], 1));
    builder.emit(abi::add_registers(s[4], s[4], s[2]));
    builder.emit(abi::load_u64(s[5], sp(), at.cursor));
    builder.emit(abi::add_registers(s[4], s[4], s[5]));
    builder.emit(abi::move_immediate(
        s[7],
        "Integer",
        &VULKAN_MAX_FRAME_BAND_WORDS.to_string(),
    ));
    builder.emit(abi::compare_registers(s[4], s[7]));
    builder.emit(abi::branch_gt(&none));
    builder.emit(abi::store_u64(s[4], sp(), at.cursor));

    // --- the header: top, height, count, table offset -----------------------------
    builder.emit(abi::store_u32(s[5], sp(), block(ITEM_BAND_START)));
    builder.emit(abi::store_u32(s[6], sp(), block(ITEM_BAND_COUNT)));
    builder.emit(abi::load_u64(s[4], sp(), at.height));
    for instruction in emit_fixed_to_f32_bits(s[4], s[4], s[8], f[0], f[7]) {
        builder.emit(instruction);
    }
    builder.emit(abi::store_u32(s[4], sp(), block(ITEM_BAND_HEIGHT)));
    builder.emit(abi::load_u64(s[4], sp(), at.reach));
    builder.emit(abi::shift_right_immediate(s[4], s[4], 1));
    builder.emit(abi::load_u64(s[7], sp(), at.ymin));
    builder.emit(abi::subtract_registers(s[7], s[7], s[4]));
    for instruction in emit_fixed_to_f32_bits(s[7], s[7], s[8], f[0], f[7]) {
        builder.emit(instruction);
    }
    builder.emit(abi::store_u32(s[7], sp(), block(ITEM_BAND_TOP)));

    // --- the table's address: mapped + BAND_BASE*4 + start*4 ----------------------
    builder.emit(abi::shift_left_immediate(s[5], s[5], 2));
    emit_state_load(builder, at.state, GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED, s[4]);
    builder.emit(abi::add_registers(s[5], s[5], s[4]));
    builder.emit(abi::move_immediate(
        s[4],
        "Integer",
        &(VULKAN_BAND_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(s[5], s[5], s[4]));
    builder.emit(abi::store_u64(s[5], sp(), at.table));

    // --- counts to zero: table[2k + 1] = 0 (s5 table, s6 bands) --------------------
    let zero_head = builder.label("vk_band_zero_head");
    let zero_done = builder.label("vk_band_zero_done");
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::move_immediate(s[7], "Integer", "0"));
    builder.emit(abi::label(&zero_head));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_ge(&zero_done));
    builder.emit(abi::shift_left_immediate(s[8], s[4], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[5]));
    builder.emit(abi::store_u32(s[7], s[8], 4));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&zero_head));
    builder.emit(abi::label(&zero_done));

    // --- count each band's edges -----------------------------------------------------
    let count_head = builder.label("vk_band_count_head");
    let count_band = builder.label("vk_band_count_band");
    let count_next = builder.label("vk_band_count_next");
    let count_done = builder.label("vk_band_count_done");
    builder.emit(abi::load_u64(s[0], sp(), at.edges));
    builder.emit(abi::load_u64(s[1], sp(), at.end));
    builder.emit(abi::load_u64(s[2], sp(), at.table));
    builder.emit(abi::label(&count_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&count_done));
    emit_band_range(builder, at);
    builder.emit(abi::label(&count_band));
    builder.emit(abi::compare_registers(s[4], s[5]));
    builder.emit(abi::branch_gt(&count_next));
    builder.emit(abi::shift_left_immediate(s[6], s[4], 3));
    builder.emit(abi::add_registers(s[6], s[6], s[2]));
    builder.emit(abi::load_u32(s[7], s[6], 4));
    builder.emit(abi::add_immediate(s[7], s[7], 1));
    builder.emit(abi::store_u32(s[7], s[6], 4));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&count_band));
    builder.emit(abi::label(&count_next));
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::branch(&count_head));
    builder.emit(abi::label(&count_done));

    // --- each band's first list entry: table[2k] = sum of the counts before it -----
    let start_head = builder.label("vk_band_start_head");
    let start_done = builder.label("vk_band_start_done");
    builder.emit(abi::load_u64(s[6], sp(), at.count));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::move_immediate(s[7], "Integer", "0"));
    builder.emit(abi::label(&start_head));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_ge(&start_done));
    builder.emit(abi::shift_left_immediate(s[8], s[4], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[2]));
    builder.emit(abi::store_u32(s[7], s[8], 0));
    builder.emit(abi::load_u32(s[5], s[8], 4));
    builder.emit(abi::add_registers(s[7], s[7], s[5]));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&start_head));
    builder.emit(abi::label(&start_done));

    // --- the list: each edge's number into each of its bands, using table[2k] as the
    // band's fill cursor (s8 = the list, s3 = the edge number) ------------------------
    let fill_head = builder.label("vk_band_fill_head");
    let fill_band = builder.label("vk_band_fill_band");
    let fill_next = builder.label("vk_band_fill_next");
    let fill_done = builder.label("vk_band_fill_done");
    builder.emit(abi::shift_left_immediate(s[8], s[6], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[2]));
    builder.emit(abi::load_u64(s[0], sp(), at.edges));
    builder.emit(abi::load_u64(s[1], sp(), at.end));
    builder.emit(abi::move_immediate(s[3], "Integer", "0"));
    builder.emit(abi::label(&fill_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&fill_done));
    emit_band_range(builder, at);
    builder.emit(abi::label(&fill_band));
    builder.emit(abi::compare_registers(s[4], s[5]));
    builder.emit(abi::branch_gt(&fill_next));
    builder.emit(abi::shift_left_immediate(s[6], s[4], 3));
    builder.emit(abi::add_registers(s[6], s[6], s[2]));
    builder.emit(abi::load_u32(s[7], s[6], 0));
    builder.emit(abi::add_immediate(s[7], s[7], 1));
    builder.emit(abi::store_u32(s[7], s[6], 0));
    builder.emit(abi::subtract_immediate(s[7], s[7], 1));
    builder.emit(abi::shift_left_immediate(s[7], s[7], 2));
    builder.emit(abi::add_registers(s[7], s[7], s[8]));
    builder.emit(abi::store_u32(s[3], s[7], 0));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&fill_band));
    builder.emit(abi::label(&fill_next));
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::add_immediate(s[3], s[3], 1));
    builder.emit(abi::branch(&fill_head));
    builder.emit(abi::label(&fill_done));

    // --- the fill cursors ran to each band's end; take the counts back off ---------
    let rewind_head = builder.label("vk_band_rewind_head");
    builder.emit(abi::load_u64(s[6], sp(), at.count));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::label(&rewind_head));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::shift_left_immediate(s[8], s[4], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[2]));
    builder.emit(abi::load_u32(s[5], s[8], 0));
    builder.emit(abi::load_u32(s[7], s[8], 4));
    builder.emit(abi::subtract_registers(s[5], s[5], s[7]));
    builder.emit(abi::store_u32(s[5], s[8], 0));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&rewind_head));

    // --- no bands: the shader loops over every edge -------------------------------
    builder.emit(abi::label(&none));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    for field in [
        ITEM_BAND_TOP,
        ITEM_BAND_HEIGHT,
        ITEM_BAND_COUNT,
        ITEM_BAND_START,
    ] {
        builder.emit(abi::store_u32(s[4], sp(), block(field)));
    }
    builder.emit(abi::store_u32(s[4], sp(), at.item + set.norm));
    builder.emit(abi::label(&done));
}

/// The bands the edge at `SCRATCH[0]` goes in: first in `SCRATCH[4]`, last in
/// `SCRATCH[5]` (inclusive), from the working state `emit_band_index` parked. Uses
/// `SCRATCH[6]`, `FP_SCRATCH[0]` and `FP_SCRATCH[7]` as temporaries.
///
/// With `top = ymin − r`: first = ⌊(ylo − ymin) / h⌋ and last = ⌊(yhi − ymin + 2r) / h⌋.
/// Both numerators are non-negative (every y is at least ymin), so the unsigned divide
/// is exact, and the last is at most ⌊span / h⌋ = bands − 1.
fn emit_band_range(builder: &mut CodeBuilder, at: &BandSlots) {
    let s = abi::SCRATCH;
    let ordered = builder.label("vk_band_ordered");
    let f = abi::FP_SCRATCH;
    for instruction in emit_f32_word_to_fixed(s[4], s[0], 4, s[6], f[0], f[7]) {
        builder.emit(instruction);
    }
    for instruction in emit_f32_word_to_fixed(s[5], s[0], 12, s[6], f[0], f[7]) {
        builder.emit(instruction);
    }
    builder.emit(abi::compare_registers(s[4], s[5]));
    builder.emit(abi::branch_le(&ordered));
    builder.emit(abi::move_register(s[6], s[4]));
    builder.emit(abi::move_register(s[4], s[5]));
    builder.emit(abi::move_register(s[5], s[6]));
    builder.emit(abi::label(&ordered));
    builder.emit(abi::load_u64(s[6], abi::stack_pointer(), at.ymin));
    builder.emit(abi::subtract_registers(s[4], s[4], s[6]));
    builder.emit(abi::subtract_registers(s[5], s[5], s[6]));
    builder.emit(abi::load_u64(s[6], abi::stack_pointer(), at.reach));
    builder.emit(abi::add_registers(s[5], s[5], s[6]));
    builder.emit(abi::load_u64(s[6], abi::stack_pointer(), at.height));
    builder.emit(abi::unsigned_divide_registers(s[4], s[4], s[6]));
    builder.emit(abi::unsigned_divide_registers(s[5], s[5], s[6]));
}
