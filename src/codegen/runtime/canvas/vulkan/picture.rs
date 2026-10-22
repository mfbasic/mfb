//! plan-176-V: the picture texel upload and the picture cache lookup. The shared constants,
//! layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// Copy a `canvas::Picture`'s texels into the frame buffer's glyph region, and name
/// the slice in the item block (bug-484) — once per distinct image per frame (bug-688).
///
/// A picture is a rectangle whose fill colour is sampled from an image, so its block is
/// the rectangle's in every field but three, which it names exactly as a glyph does:
/// width in `misc.w`, height in `arc.x` (`ITEM_ARC_GLYPH_HEIGHT`), and the slice's base
/// in `arc.z` (`ITEM_ARC_EDGE_BASE`). The region, the cursor and the frame cap are the
/// glyphs' — the predicates count a picture's texels against that same cap.
///
/// **Once per distinct pixel block** (bug-688, Metal's bug-686 design): the block is
/// looked up in this frame's picture table first (`emit_picture_lookup`), and a hit
/// points the item at the texels already uploaded and copies nothing. A tilemap of 2,000
/// tiles drawn from two images copied 2,000 images and counted 2M texels against the
/// frame cap; it now copies two. The pass walk (`__canvas_passNext`) counts each
/// distinct block once a pass to match, and the table is a pass's: each pass is one
/// call of this renderer, which starts it empty.
///
/// **Must run after `emit_edge_upload`**, which zeroes `misc.w` and `arc.z` for every
/// kind that is not a polygon, and **before `emit_split_or_publish`**, so both records
/// of a split item carry the slice. Unlike a glyph run, a picture is one quad and does
/// not end the instanced run.
///
/// One 32-bit word per texel, copied verbatim: the pixel block is a `List OF Byte` of
/// RGBA8, so each little-endian word already reads `r | g << 8 | b << 16 | a << 24`,
/// the packing the shader unpacks. The block's address arrives split across two header
/// doubles (`HEADER_PICTURE_SHADOW_HI`/`_LO`) because a whole address overflows the
/// header hash; the texels start `COLLECTION_HEADER_SIZE` bytes in. The block is never
/// freed, so reading it here, during the frame, is safe. A zero address or a block
/// shorter than the width and height promise draws nothing — the bound
/// `canvas::shadowTexel` applies per read, applied once, as Metal's emitter does.
///
/// A picture whose texels would not fit the frame's remaining glyph room draws nothing
/// (`misc.w = 0`), rather than writing past the region. Unreachable: the predicate
/// declined such a frame to software.
///
/// Registers: `SCRATCH[0..=8]` only, and no calls. No `compare_immediate` or
/// `add_immediate` against a constant wider than 12 bits while `SCRATCH[7]` or
/// `SCRATCH[8]` is live: on AArch64 their encoding borrows x16/x17, which ARE those two.
pub(super) fn emit_picture_upload(builder: &mut CodeBuilder, at: PictureSlots) {
    let done = builder.label("vk_pic_done");
    let empty = builder.label("vk_pic_empty");
    let copy_head = builder.label("vk_pic_copy_head");

    let header = abi::SCRATCH[6];
    let width = abi::SCRATCH[2];
    let height = abi::SCRATCH[3];
    let samples = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let block = abi::SCRATCH[7];

    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_PICTURE));
    let image = builder.label("vk_pic_image");
    builder.emit(abi::branch_eq(&image));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_TILE));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::label(&image));

    builder.emit(abi::load_u64(header, abi::stack_pointer(), at.header));
    // A Tile reuses the gradient words for its source rectangle. Picture never reads
    // them, and Tile has no gradient, so the per-kind alias does not widen ItemBlock.
    let source_done = builder.label("vk_tile_source_done");
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_TILE));
    builder.emit(abi::branch_ne(&source_done));
    for (index, slot) in [TILE_SOURCE_X, TILE_SOURCE_Y, TILE_SOURCE_W, TILE_SOURCE_H]
        .into_iter()
        .enumerate()
    {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        for instruction in emit_f32_bits(abi::SCRATCH[1], abi::FP_SCRATCH[1]) {
            builder.emit(instruction);
        }
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            at.item + ITEM_OFFSET_GRADIENT + index * 4,
        ));
    }
    builder.emit(abi::label(&source_done));
    for (slot, register) in [(HEADER_AUX0, width), (HEADER_AUX1, height)] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_convert_to_signed_x(register, abi::FP_SCRATCH[1]));
    }
    builder.emit(abi::compare_immediate(width, "0"));
    builder.emit(abi::branch_le(&empty));
    builder.emit(abi::compare_immediate(height, "0"));
    builder.emit(abi::branch_le(&empty));
    builder.emit(abi::multiply_registers(samples, width, height));

    // block = (hi << PICTURE_SHADOW_SPLIT_BITS) | lo
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_SHADOW_HI * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(block, abi::FP_SCRATCH[1]));
    builder.emit(abi::shift_left_immediate(
        block,
        block,
        PICTURE_SHADOW_SPLIT_BITS as u8,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_SHADOW_LO * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::or_registers(block, block, abi::SCRATCH[1]));
    builder.emit(abi::compare_immediate(block, "0"));
    builder.emit(abi::branch_eq(&empty));
    // The block must hold every texel the width and height promise.
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        block,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::shift_left_immediate(abi::SCRATCH[0], samples, 2));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_gt(&empty));

    // The height is the image's whatever the lookup finds, so it is stored now and its
    // register is free for the lookup.
    builder.emit(abi::store_u32(
        height,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    // Has this frame already uploaded this block? Then name those texels and stop.
    emit_picture_lookup(builder, &at, &done);

    // Room check against the glyph region's frame cap — the same bound the glyph copy
    // applies, through a register because the cap does not fit an immediate compare.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        samples,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[8],
        "Integer",
        &VULKAN_MAX_FRAME_GLYPH_SAMPLES.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[8]));
    builder.emit(abi::branch_gt(&empty));

    // The slice: width and the pre-advance cursor (the height is already stored).
    builder.emit(abi::store_u32(
        width,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    // Record (block, base) so later pictures of this block reuse the texels. The base is
    // still in `SCRATCH[0]`.
    emit_picture_record(builder, &at);

    // target = mapped + (GLYPH_BASE + base) * 4, the base being the advanced cursor less
    // `samples` (the record used the registers that held it).
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::subtract_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        samples,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &VULKAN_GLYPH_BASE_WORDS.to_string(),
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        2,
    ));
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[0]));
    builder.emit(abi::add_immediate(block, block, COLLECTION_HEADER_SIZE));

    // `samples` counts down; it is at least 1 here.
    builder.emit(abi::label(&copy_head));
    builder.emit(abi::load_u32(abi::SCRATCH[0], block, 0));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
    builder.emit(abi::add_immediate(block, block, 4));
    builder.emit(abi::add_immediate(target, target, 4));
    builder.emit(abi::subtract_immediate(samples, samples, 1));
    builder.emit(abi::compare_immediate(samples, "0"));
    builder.emit(abi::branch_gt(&copy_head));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for offset in [
        ITEM_OFFSET_MISC + 12,
        ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
        ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.item + offset,
        ));
    }

    builder.emit(abi::label(&done));
}

/// The stack slots `emit_picture_upload` and its table helpers read.
pub(super) struct PictureSlots {
    pub(super) state: usize,
    pub(super) item: usize,
    pub(super) header: usize,
    pub(super) glyph_cursor: usize,
    /// This frame's picture-table record count, zeroed with the other cursors.
    pub(super) pic_count: usize,
    /// The empty index slot a missed lookup ended at, for `emit_picture_record`.
    pub(super) pic_slot: usize,
}

/// The picture table's multiplicative hash constant (xorshift*'s) — the one Metal's
/// table uses — odd so the multiply is a bijection on the address bits it keeps.
const PICTURE_HASH_MULTIPLIER: &str = "2685821657736338717";

/// Look the picture's pixel block up in this frame's picture table (bug-688; the layout
/// is at `VULKAN_PICTURE_RECORDS_OFFSET`, Metal's design). On a hit, name the uploaded
/// texels in the item block (base in `arc.z`, width in `misc.w`) and branch to `done`;
/// on a miss, park the empty index slot the probe ended at in `pic_slot` and fall
/// through to the upload.
///
/// Live on entry: `block` (`SCRATCH[7]`), `width` (`SCRATCH[2]`), `samples`
/// (`SCRATCH[4]`). Uses `SCRATCH[0,1,3,5,6,8]`.
///
/// A slot is live only when its record number is below this frame's count and that
/// record names the slot back — the sparse-set test — so the index never needs clearing,
/// and whatever the mapped memory held before the first frame reads as empty.
fn emit_picture_lookup(builder: &mut CodeBuilder, at: &PictureSlots, done: &str) {
    let probe = builder.label("vk_pic_probe");
    let live = builder.label("vk_pic_live");
    let miss = builder.label("vk_pic_miss");
    let hit = builder.label("vk_pic_hit");
    let (index, record_no, record, records, slot, count) = (
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[8],
    );
    let (width, block) = (abi::SCRATCH[2], abi::SCRATCH[7]);
    const INDEX_BITS: u32 = VULKAN_PICTURE_INDEX_SLOTS.trailing_zeros();

    // records = mapped + RECORDS_OFFSET; index = mapped + INDEX_OFFSET.
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        records,
    );
    builder.emit(abi::move_immediate(
        index,
        "Integer",
        &VULKAN_PICTURE_INDEX_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(index, records, index));
    builder.emit(abi::move_immediate(
        record_no,
        "Integer",
        &VULKAN_PICTURE_RECORDS_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(records, records, record_no));
    // slot = ((block >> 4) * K) >> (64 - INDEX_BITS). Blocks are 16-byte aligned arena
    // allocations, so the low four bits carry nothing.
    builder.emit(abi::shift_right_immediate(slot, block, 4));
    builder.emit(abi::move_immediate(
        record_no,
        "Integer",
        PICTURE_HASH_MULTIPLIER,
    ));
    builder.emit(abi::multiply_registers(slot, slot, record_no));
    builder.emit(abi::shift_right_immediate(
        slot,
        slot,
        (64 - INDEX_BITS) as u8,
    ));
    builder.emit(abi::load_u64(count, abi::stack_pointer(), at.pic_count));

    builder.emit(abi::label(&probe));
    builder.emit(abi::shift_left_immediate(record_no, slot, 2));
    builder.emit(abi::add_registers(record_no, index, record_no));
    builder.emit(abi::load_u32(record_no, record_no, 0));
    // Unsigned: a slot holding garbage from before this buffer's first frame is a large
    // number, and it must read as "not below the count" rather than as negative.
    builder.emit(abi::compare_registers(record_no, count));
    builder.emit(abi::branch_lo(&live));
    builder.emit(abi::branch(&miss));
    builder.emit(abi::label(&live));
    const _: () = assert!(
        PICTURE_RECORD_BYTES == 16,
        "records are addressed by a shift of 4"
    );
    builder.emit(abi::shift_left_immediate(record, record_no, 4));
    builder.emit(abi::add_registers(record, records, record));
    builder.emit(abi::load_u32(record_no, record, 12));
    builder.emit(abi::compare_registers(record_no, slot));
    builder.emit(abi::branch_ne(&miss));
    builder.emit(abi::load_u64(record_no, record, 0));
    builder.emit(abi::compare_registers(record_no, block));
    builder.emit(abi::branch_eq(&hit));
    // Another block's live slot: the next one, wrapping.
    builder.emit(abi::add_immediate(slot, slot, 1));
    builder.emit(abi::move_immediate(
        record_no,
        "Integer",
        &(VULKAN_PICTURE_INDEX_SLOTS - 1).to_string(),
    ));
    builder.emit(abi::and_registers(slot, slot, record_no));
    builder.emit(abi::branch(&probe));

    builder.emit(abi::label(&hit));
    builder.emit(abi::load_u32(record_no, record, 8));
    builder.emit(abi::store_u32(
        record_no,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    builder.emit(abi::store_u32(
        width,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::branch(done));

    builder.emit(abi::label(&miss));
    builder.emit(abi::store_u64(slot, abi::stack_pointer(), at.pic_slot));
}

/// Record the picture just given its slice of the glyph region — `block` in
/// `SCRATCH[7]`, its base (the pre-advance glyph cursor) in `SCRATCH[0]` — at the index
/// slot `emit_picture_lookup` parked (bug-688).
///
/// Uses `SCRATCH[0,1,3,5,6]`; `block`, `width` and `samples` survive for the copy.
/// The count check is unreachable — one record per picture item, and the predicate caps
/// items at `VULKAN_PICTURE_RECORDS` — and is kept because the alternative is a write
/// past the table; an unrecorded picture is still uploaded, only not shared.
fn emit_picture_record(builder: &mut CodeBuilder, at: &PictureSlots) {
    let full = builder.label("vk_pic_table_full");
    let (base, count, entry, temp, mapped) = (
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    );
    let block = abi::SCRATCH[7];

    builder.emit(abi::load_u64(count, abi::stack_pointer(), at.pic_count));
    builder.emit(abi::move_immediate(
        temp,
        "Integer",
        &VULKAN_PICTURE_RECORDS.to_string(),
    ));
    builder.emit(abi::compare_registers(count, temp));
    builder.emit(abi::branch_ge(&full));

    // records[count] = (block, base, slot)
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        mapped,
    );
    builder.emit(abi::shift_left_immediate(entry, count, 4));
    builder.emit(abi::add_registers(entry, mapped, entry));
    builder.emit(abi::move_immediate(
        temp,
        "Integer",
        &VULKAN_PICTURE_RECORDS_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(entry, entry, temp));
    builder.emit(abi::store_u64(block, entry, 0));
    builder.emit(abi::store_u32(base, entry, 8));
    builder.emit(abi::load_u64(temp, abi::stack_pointer(), at.pic_slot));
    builder.emit(abi::store_u32(temp, entry, 12));
    // index[slot] = count -- written AFTER the record, so the slot is never live while
    // the record it names is stale.
    builder.emit(abi::shift_left_immediate(temp, temp, 2));
    builder.emit(abi::add_registers(entry, mapped, temp));
    builder.emit(abi::move_immediate(
        temp,
        "Integer",
        &VULKAN_PICTURE_INDEX_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(entry, entry, temp));
    builder.emit(abi::store_u32(count, entry, 0));
    builder.emit(abi::add_immediate(count, count, 1));
    builder.emit(abi::store_u64(count, abi::stack_pointer(), at.pic_count));
    builder.emit(abi::label(&full));
}
