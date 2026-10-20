//! plan-176-V: item publication — split or publish, the draw-list pass and glyph runs. The
//! shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// Copy the item block just built on the stack into the frame's item buffer at the
/// cursor, and advance the cursor by one quad.
///
/// This is what replaced `vkCmdPushConstants` (plan-116-A). The block is built on the
/// stack exactly as before — `emit_item_block` and `emit_edge_upload` are untouched —
/// and then lands in the buffer instead of in the command stream, at the index the
/// shaders will read it back from through `gl_InstanceIndex`.
///
/// **The cursor counts quads, not scene items.** A shape is one, and a glyph run is
/// one per glyph, because each glyph is its own quad with its own block. That is the
/// same number `__canvas_blockCost` sums against `VULKAN_MAX_FRAME_ITEMS`, so
/// the two cannot disagree about what "full" means.
///
/// Branches to `full` without writing or advancing if the buffer is already at
/// capacity. Unreachable in the same way `emit_edge_upload`'s bound is: the pass walk
/// ends a pass before its quads pass the region (bug-765). Kept because the
/// alternative is a write past the mapping.
fn emit_item_publish(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_item_cursor: usize,
    full: &str,
) {
    let cursor = abi::SCRATCH[0];
    let target = abi::SCRATCH[1];
    let value = abi::SCRATCH[2];
    let stride = abi::SCRATCH[3];

    builder.emit(abi::load_u64(cursor, abi::stack_pointer(), off_item_cursor));
    builder.emit(abi::compare_immediate(
        cursor,
        &VULKAN_MAX_FRAME_ITEMS.to_string(),
    ));
    builder.emit(abi::branch_ge(full));

    // target = mapped + cursor * ITEM_BLOCK_SIZE. A multiply rather than a second
    // byte-cursor kept in step with this one: two cursors that must never diverge is
    // exactly the invariant that breaks silently, and the product is one instruction.
    builder.emit(abi::move_immediate(
        stride,
        "Integer",
        &ITEM_BLOCK_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(stride, cursor, stride));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_ITEM_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, stride));

    // 112 bytes is 14 words. Unrolled: a loop would need a counter register that no
    // call clobbers, for fourteen iterations.
    // bug-550: a `const` assertion, not a `debug_assert_eq!`. Both operands are
    // compile-time constants, so this is decidable when the compiler ITSELF is
    // built and costs nothing at run time — strictly better than an assertion that
    // ran nowhere because CI builds `--release`.
    const _: () = assert!(
        ITEM_BLOCK_SIZE % 8 == 0,
        "the item block is copied to the buffer eight bytes at a time"
    );
    for word in 0..ITEM_BLOCK_SIZE / 8 {
        builder.emit(abi::load_u64(
            value,
            abi::stack_pointer(),
            off_item + word * 8,
        ));
        builder.emit(abi::store_u64(value, target, word * 8));
    }

    builder.emit(abi::add_immediate(cursor, cursor, 1));
    builder.emit(abi::store_u64(
        cursor,
        abi::stack_pointer(),
        off_item_cursor,
    ));
}

/// Publish this item's block — as **two** records when the fragment shader's
/// stroke-over-fill composition would not equal the oracle's two sequential blends.
///
/// The shaders compose stroke over fill *in the shader* and hand the hardware one
/// source, and `mfb_canvas.frag` states the identity that rests on: it equals the
/// oracle's two writes because `over` is associative. **That is `Normal`-only.** The
/// oracle applies the mode twice per pixel — fill into the surface, then stroke into
/// the result — and `M(M(D, fill), stroke) = M(D, over(stroke, fill))` holds for
/// `over` and for none of `Multiply`, `Screen` or `Add` wherever the stroke band
/// covers filled pixels.
///
/// So an item that is non-`Normal` **and** both fills and strokes becomes two adjacent
/// records: the first with `strokeHalf` zeroed (fill only), the second with the fill
/// alpha zeroed (stroke only), in that order. Each then reaches the fixed-function
/// unit as a single source, and paint order is exactly the oracle's. The fragment
/// shader needs no change for it — a zero `strokeHalf` skips the stroke arm and a zero
/// fill alpha premultiplies to nothing.
///
/// Everything else stays one record: `Normal` items, fill-only items and stroke-only
/// items all take the single-publish path they had.
pub(super) fn emit_split_or_publish(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_item_cursor: usize,
    off_item_mode: usize,
    off_saved_stroke: usize,
    full: &str,
) {
    let single = builder.label("vk_publish_single");
    let done = builder.label("vk_publish_done");
    let strokes = builder.label("vk_publish_strokes");

    // Split only when the mode is not Normal...
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item_mode,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&single));
    // ...and the item actually strokes: `strokeHalf` > 0, which since bug-735 is its
    // 16.16 word > 0 or, for a word of 0, its residual > 0 -- `emit_item_stroke_sign`
    // makes that exactly the header's f64 `half > 0.0`, the test
    // `__canvas_blockInstances` counts two records on. Before it this read the word
    // alone, so a blended filled item stroked thinner than 2^-16 px was counted as two
    // records and published as one, and every later draw named the wrong blocks.
    //
    // SIGNED, and that is the whole point. `__canvas_strokeHalf` returns **-1.0** for a
    // paint that does not stroke, which is `0xFFFF0000` in 16.16 -- and `load_u32`
    // zero-extends, so the 64-bit compare below saw 4294901760 and every blended
    // fill-only item took the SPLIT path. The comment under `single` has always claimed
    // such an item is fill-only and takes the single path; this is the load that makes
    // it true.
    //
    // It stayed invisible until plan-116-H. While the draws came from `emit_run_flush`
    // the instance count was `cursor - run_start` -- whatever had actually been
    // published -- so the spurious second record was drawn and painted nothing. Once the
    // draw list predicts instance counts independently (`__canvas_blockInstances`), one
    // extra record shifts every later base by one and the scene silently loses its tail.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    builder.emit(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_gt(&strokes));
    builder.emit(abi::branch_lt(&single));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC_LO + 4,
    ));
    builder.emit(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_le(&single));
    builder.emit(abi::label(&strokes));
    // ...and actually fills (fill alpha > 0). A `Line` or an `Arc` reaches here with
    // its stroke colour already moved into the fill slots and `strokeHalf` negative
    // (`__canvas_strokeAsFill`), so it is fill-only and takes the single path.
    //
    // plan-163-D: "fills" is a visible fill alpha, OR a gradient (its ramp is the fill
    // colour whatever the fill alpha), OR a picture (its image is, and since plan-163-C
    // its fill alpha is only the tint's strength). Testing the fill alpha alone sent a
    // blended stroked gradient or picture with a transparent fill down the single path,
    // composing stroke over fill in the shader under a mode where that is wrong.
    let fills = builder.label("vk_publish_fills");
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_FILL + 12,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_gt(&fills));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "2"));
    builder.emit(abi::branch_ge(&fills));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        &GEO_KIND_PICTURE.to_string(),
    ));
    builder.emit(abi::branch_ne(&single));
    builder.emit(abi::label(&fills));

    // Record one: the fill, with the stroke switched off.
    //
    // `strokeHalf` is parked on the STACK, not in a scratch register.
    // `emit_item_publish` uses `SCRATCH[0..3]` — `SCRATCH[1]` is its target pointer —
    // so a register saved across it comes back holding a mapped address. Measured
    // rather than reasoned about: doing exactly that gave record two a `strokeHalf` of
    // whatever the buffer pointer was, i.e. an enormous stroke band, and the Vulkan
    // harness reported `worst=215` with the GPU painting stroke pixels the oracle
    // never touched.
    //
    // Off is -1.0 (`0xFFFF0000`), the value `__canvas_strokeHalf` reports for a paint
    // that does not stroke: since bug-735 the shader strokes on the word PLUS its
    // residual, and a zero word under a positive residual would still stroke. Spelled
    // unsigned: the store keeps the low 32 bits.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_saved_stroke,
    ));
    // -1.0 in 16.16, spelled unsigned: the immediate is never negative, and the store
    // keeps the low 32 bits.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        "4294901760",
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    emit_item_publish(builder, off_state, off_item, off_item_cursor, full);
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_saved_stroke,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));

    // Record two: the stroke, with the fill made fully transparent.
    // plan-163-D: marked OFF -- alpha -1, which every shader reads as "no fill in this
    // record" -- rather than made transparent: a gradient's or a picture's colour does
    // not come from the fill alpha, so a zero there left record two drawing the ramp or
    // the image a second time under the blend. The stroke still reads the ramp
    // (`FillAndStroke`). Spelled unsigned; the store keeps the low 32 bits.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        "4294967295",
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_FILL + 12,
    ));
    emit_item_publish(builder, off_state, off_item, off_item_cursor, full);
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&single));
    emit_item_publish(builder, off_state, off_item, off_item_cursor, full);
    builder.emit(abi::label(&done));
}

/// Draw every quad published since the last flush as **one instanced draw**, and start
/// a new run.
///
/// Walk `__CANVAS_DRAWS` and issue one instanced `vkCmdDraw` per entry, pushing that
/// entry's group offset first (plan-116-H).
///
/// **This is the second of the emitter's two passes** (**H9**). The first fills the item
/// buffer, walking the block list once so a shared group's blocks are written once; this
/// one walks the draw list, which may visit the same block range several times — a
/// diamond is two entries with the same base. No single forward pass can do both, which
/// is what the Phase 1 sharing decision costs and buys.
///
/// Every entry is one instanced call, text included. A glyph run is N contiguous blocks
/// in the item buffer, one per quad, so `instanceCount = N` draws them all — the
/// "a glyph run is N draws by design" shape predates plan-116-A putting the block in a
/// buffer indexed by instance, and the draw list's counts are already in instances.
///
/// Each entry is four `i32`s: `base`, `count`, `dx`, `dy` — the offsets in 16.16, which
/// is the form the push constant wants, so they travel from MFBASIC to the shader
/// without a conversion on either side.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_draw_list_pass(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_state: usize,
    off_handle: usize,
    off_cmd_handle: usize,
    off_draw_fn: usize,
    off_draws: usize,
    off_draw_entries: usize,
    unavailable: &str,
) -> Result<(), String> {
    let head = builder.label("vk_drawlist_head");
    let done = builder.label("vk_drawlist_done");

    let off_push_fn = builder.allocate_stack_object("vk_push_fn", 8);
    let off_index = builder.allocate_stack_object("vk_drawlist_index", 8);
    // The two 16.16 words, their residual words and (plan-172-H) the four words of the
    // inverse linear part, contiguous, so `pValues` can point straight at them.
    let off_offset_pair = builder.allocate_stack_object("vk_push_offset", PUSH_OFFSET_BYTES);

    let off_bind_fn = builder.allocate_stack_object("vk_drawlist_bind_fn", 8);
    for (name, slot) in [
        ("vkCmdPushConstants", off_push_fn),
        ("vkCmdBindPipeline", off_bind_fn),
    ] {
        emit_dlsym(
            builder,
            platform,
            platform_imports,
            name,
            off_handle,
            slot,
            unavailable,
        )?;
    }

    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), off_index));
    builder.emit(abi::label(&head));
    let index = builder.temporary_vreg();
    let total = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, abi::stack_pointer(), off_index));
    builder.emit(abi::load_u64(
        &total,
        abi::stack_pointer(),
        off_draw_entries,
    ));
    builder.emit(abi::compare_registers(&index, &total));
    builder.emit(abi::branch_ge(&done));

    // entry = draws + index * (CANVAS_DRAW_ENTRY_WORDS * 8)
    let entry = builder.temporary_vreg();
    builder.emit(abi::load_u64(&entry, abi::stack_pointer(), off_draws));
    // Sixteen elements per entry, eight bytes each. An MFBASIC `Integer` is 64-bit,
    // which is the whole reason the fields below are at 0/8/16/24/32 and are loaded with
    // `load_u64`: reading them as 32-bit words takes the low half of `base` and the low
    // half of `count` as if they were `base` and `dx`, which draws real geometry at
    // nonsense instance indices -- 35% of the frame wrong, worst=255, with a draw list
    // that every assertion says is correct.
    let scaled = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(
        &scaled,
        &index,
        CANVAS_DRAW_ENTRY_SHIFT as u8,
    ));
    builder.emit(abi::add_registers(&entry, &entry, &scaled));

    // The offset pair, copied into its own slot so `pValues` has a stable address that
    // is not part of a collection another thread may be writing.
    let word = builder.temporary_vreg();
    builder.emit(abi::load_u64(&word, &entry, 16));
    builder.emit(abi::store_u32(&word, abi::stack_pointer(), off_offset_pair));
    builder.emit(abi::load_u64(&word, &entry, 24));
    builder.emit(abi::store_u32(
        &word,
        abi::stack_pointer(),
        off_offset_pair + 4,
    ));

    // The base and count are staged through the stack for the reason `emit_run_flush`
    // records: on x86-64 the scratch pool aliases the C argument bank, so a value held
    // in a scratch register across the staging is one an earlier argument destroys.
    let off_base = builder.allocate_stack_object("vk_drawlist_base", 8);
    let off_count = builder.allocate_stack_object("vk_drawlist_count", 8);
    builder.emit(abi::load_u64(&word, &entry, 0));
    builder.emit(abi::store_u64(&word, abi::stack_pointer(), off_base));
    builder.emit(abi::load_u64(&word, &entry, 8));
    builder.emit(abi::store_u64(&word, abi::stack_pointer(), off_count));

    // Bind THIS entry's pipeline. The publish walk binds as it goes, but every draw now
    // happens after that walk has finished, so a binding left there would apply the last
    // item's mode to the whole frame — measured at 35.1% of pixels wrong before the mode
    // travelled with the entry.
    // handle = *(state + …_PIPELINE_MODES + mode * 8) — the same shift-and-add the
    // publish walk uses, so the two cannot pick different pipelines for one mode.
    //
    // In vregs, not `SCRATCH[k]`: `entry` is a vreg live across this stretch, and the
    // allocator does not see hand-assigned scratch, so on x86-64 it may put `entry` in
    // the very register a fixed `SCRATCH[k]` names (bug-688 measured that collision in
    // `emit_state_load`).
    let off_pipeline = builder.allocate_stack_object("vk_drawlist_pipeline", 8);
    let state = builder.temporary_vreg();
    builder.emit(abi::load_u64(&word, &entry, CANVAS_DRAW_ENTRY_MODE));
    builder.emit(abi::shift_left_immediate(&word, &word, 3));
    builder.emit(abi::load_u64(&state, abi::stack_pointer(), off_state));
    builder.emit(abi::add_registers(&word, &state, &word));
    builder.emit(abi::load_u64(
        &word,
        &word,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES,
    ));
    builder.emit(abi::store_u64(&word, abi::stack_pointer(), off_pipeline));
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    builder.emit(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_pipeline,
    ));
    emit_call_fn(builder, off_bind_fn);

    // Now the offset pair, (bug-735) its residual words, entry words 5 and 6, and
    // (plan-172-H) the inverse linear part, entry words 8-11, which are zero in an entry
    // with no matrix -- into the slot the bind above borrowed.
    for (word_index, at) in [
        (2usize, 0usize),
        (3, 4),
        (5, 8),
        (6, 12),
        (8, 16),
        (9, 20),
        (10, 24),
        (11, 28),
    ] {
        builder.emit(abi::load_u64(word, entry, word_index * 8));
        builder.emit(abi::store_u32(
            word,
            abi::stack_pointer(),
            off_offset_pair + at,
        ));
    }

    // vkCmdPushConstants(cmd, layout, stageFlags, offset, size, pValues)
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        SHADER_STAGE_VERTEX_AND_FRAGMENT,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    // Arguments five and six go through the register-model helpers, not through
    // `c_arg(4)` and `c_arg(5)` directly. `has_vulkan_backend` is true for **Windows**
    // as well as Linux, and Win64 passes only four integer arguments in registers — so
    // a raw `c_arg(4)` there writes a register the callee never reads and leaves the
    // real stack slot undefined. This is the only six-argument call this pass makes,
    // and it is the reason `emit_run_flush` reached for `emit_int_arg_slot` on its
    // fifth argument rather than writing the register.
    emit_int_arg(builder, platform, 4, &PUSH_OFFSET_BYTES.to_string());
    emit_addr_arg(builder, platform, 5, off_offset_pair);
    emit_call_fn(builder, off_push_fn);

    // vkCmdDraw(cmd, 4, count, 0, base)
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "4"));
    builder.emit(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_int_arg_slot(builder, platform, 4, off_base);
    emit_call_fn(builder, off_draw_fn);

    let bumped = builder.temporary_vreg();
    builder.emit(abi::load_u64(&bumped, abi::stack_pointer(), off_index));
    builder.emit(abi::add_immediate(&bumped, &bumped, 1));
    builder.emit(abi::store_u64(&bumped, abi::stack_pointer(), off_index));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&done));
    Ok(())
}

/// `canvas::vulkanDrawScene(surface, width, height, geometry, offsets)` — render one
/// frame on the GPU and read it back into `surface`.
///
/// The Vulkan counterpart of `canvas::metalDrawScene`, and deliberately the same
/// shape: draw offscreen, read back, and let the frame leave through the same
/// `canvas::blitSurface` every other path uses. That is what makes the two backends
/// and the software oracle comparable at all — the tolerance comparator diffs an
/// RGBA8 buffer — and here it is also what makes the renderer testable headless.
///
/// This member always reads back. The direct present is `canvas::vulkanPresentScene`
/// (`vulkan/present/`), which is Windows only: on Linux a frame is read back even in a
/// window.
///
/// The submit is followed by `vkQueueWaitIdle`, so the frame is complete before this
/// returns and `canvas::frameDone` advances D's counter after real GPU completion —
/// the same ordering plan-98-E Correction 12 records for Metal, and for the same
/// reason: the readback is on the critical path, so a fence-and-continue would be
/// weaker, not stronger.
/// The stack slots `emit_glyph_draws` reads and writes.
///
/// A struct rather than seventeen parameters, because every one of them is a `usize`
/// stack offset and a transposed pair would compile, run, and draw the wrong thing.
pub(super) struct GlyphPublishSlots {
    pub(super) state: usize,
    pub(super) item: usize,
    pub(super) header: usize,
    pub(super) width: usize,
    pub(super) height: usize,
    /// The frame's item-buffer cursor — shared with the shape loop, because a glyph is
    /// a quad and takes a block exactly as a shape does.
    pub(super) item_cursor: usize,
    /// Where this glyph's block landed, parked so the draw's `firstInstance` can be
    /// staged from memory rather than held in a scratch register across the staging.
    pub(super) instance: usize,
    pub(super) glyph_meta: usize,
    pub(super) glyph_cov: usize,
    pub(super) glyph_cursor: usize,
    pub(super) glyph_index: usize,
    pub(super) glyph_count: usize,
    pub(super) glyph_w: usize,
    pub(super) glyph_h: usize,
    pub(super) glyph_x: usize,
    pub(super) glyph_y: usize,
}

/// One quad per glyph, for a `__CANVAS_GEO_TEXT` item.
///
/// **Why a glyph is not a shape.** Every other kind is one draw whose fragment shader
/// evaluates a signed distance. A glyph has no distance to evaluate: the CPU already
/// rasterised its outline into a coverage bitmap and cached it, which is the whole point
/// of plan-98-G Phase 2 — the alternative costs `O(edges)` per pixel and measured 8.1
/// seconds for twelve characters. So the GPU's job is a *lookup*, and the unit of work
/// is the glyph rather than the run.
///
/// The bitmap travels the same way a polygon's edges do: copied into the frame's shared
/// buffer at a running cursor, with the offset in the item block. One sample per 32-bit
/// word — the region is sized for it, and the arm that reads it has no shifting to get
/// wrong (`VULKAN_MAX_FRAME_GLYPH_SAMPLES`).
///
/// Everything the loop carries lives on the stack, not in registers. Two calls happen
/// per glyph, and on x86-64 the scratch pool aliases the C argument bank — so a loop
/// counter in a register is a loop counter the next call overwrites (`.ai/arch-abi.md`).
pub(super) fn emit_glyph_publish(builder: &mut CodeBuilder, at: GlyphPublishSlots) {
    let head = builder.label("vk_glyph_head");
    let done = builder.label("vk_glyph_done");
    let next = builder.label("vk_glyph_next");
    let blank = builder.label("vk_glyph_blank");
    let publish = builder.label("vk_glyph_publish");
    let copy_head = builder.label("vk_glyph_copy_head");
    let copy_done = builder.label("vk_glyph_copy_done");

    // The run's glyph count, and the colour/surface fields, come from the item block the
    // header describes — built once for the run and then edited per glyph, because fill,
    // stroke and surface are the same for every glyph in it.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.header,
    ));
    emit_item_block(builder, at.item, at.width, at.height);

    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.header,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
        HEADER_AUX0 * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_count,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_index,
    ));

    // kind = 6, radius = 0, strokeHalf = 0. A glyph is fill-only: a stroked text item
    // became an outline polygon in the geometry builder, so it never reaches here.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        GEO_KIND_TEXT,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in 1..3 {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.item + ITEM_OFFSET_MISC + word * 4,
        ));
    }

    builder.emit(abi::label(&head));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_index,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_count,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_ge(&done));

    // run = header + HEADER_SLOTS + index * GLYPH_RUN_SLOTS, in doubles.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &GLYPH_RUN_SLOTS.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.header,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        HEADER_SLOTS * 8,
    ));

    // entry, penX, penY.
    for (slot, register) in [
        (0usize, abi::SCRATCH[2]),
        (1, abi::SCRATCH[3]),
        (2, abi::SCRATCH[4]),
    ] {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[0],
            slot * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(register, abi::FP_SCRATCH[1]));
    }
    // A cache entry of -1 is a glyph the eviction pass dropped after this run was
    // built. It draws nothing rather than reading the metadata list out of range, and
    // still publishes its (blank) block — see `blank` below.
    builder.emit(abi::compare_immediate(abi::SCRATCH[2], "0"));
    builder.emit(abi::branch_lt(&blank));

    // meta = glyphMeta + entry * GLYPH_META_SLOTS, in 8-byte Integers.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        &GLYPH_META_SLOTS.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[5],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_meta,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[5],
        abi::SCRATCH[2],
    ));

    // x = x0 + penX, y = y0 + penY; w, h; and the coverage start, parked for the copy.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_X0 * 8,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[3],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_x,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_Y0 * 8,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[4],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_y,
    ));
    for (slot, parked) in [(GLYPH_META_W, at.glyph_w), (GLYPH_META_H, at.glyph_h)] {
        builder.emit(abi::load_u64(abi::SCRATCH[5], abi::SCRATCH[2], slot * 8));
        builder.emit(abi::store_u64(
            abi::SCRATCH[5],
            abi::stack_pointer(),
            parked,
        ));
    }
    // An empty bitmap — a space, or a glyph with no contours — has nothing to copy and
    // nothing to draw, but still publishes its (blank) block — see `blank` below.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[5], "0"));
    builder.emit(abi::branch_le(&blank));
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[6], "0"));
    builder.emit(abi::branch_le(&blank));

    // samples = w * h, and the frame's remaining room for them. The predicate has
    // already declined a frame that does not fit, so this bound is the emitter refusing
    // to write past its buffer if the two ever disagree — not a policy of its own.
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[8],
        abi::SCRATCH[7],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[9],
        "Integer",
        &VULKAN_MAX_FRAME_GLYPH_SAMPLES.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[8], abi::SCRATCH[9]));
    builder.emit(abi::branch_gt(&blank));

    // --- copy the bitmap into the buffer's glyph region --------------------------
    // dst = mapped + (GLYPH_BASE + cursor) * 4, src = coverage + covStart.
    builder.emit(abi::load_u64(
        abi::SCRATCH[8],
        abi::SCRATCH[2],
        GLYPH_META_START * 8,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[9],
        abi::stack_pointer(),
        at.glyph_cov,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[8],
        abi::SCRATCH[9],
        abi::SCRATCH[8],
    ));
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        abi::SCRATCH[9],
    );
    builder.emit(abi::add_immediate(
        abi::SCRATCH[10],
        abi::SCRATCH[7],
        VULKAN_GLYPH_BASE_WORDS,
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[10],
        abi::SCRATCH[10],
        2,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[9],
        abi::SCRATCH[9],
        abi::SCRATCH[10],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[10], "Integer", "0"));
    builder.emit(abi::label(&copy_head));
    builder.emit(abi::compare_registers(abi::SCRATCH[10], abi::SCRATCH[6]));
    builder.emit(abi::branch_ge(&copy_done));
    builder.emit(abi::add_registers(
        abi::SCRATCH[11],
        abi::SCRATCH[8],
        abi::SCRATCH[10],
    ));
    builder.emit(abi::load_u8(abi::SCRATCH[11], abi::SCRATCH[11], 0));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[12],
        abi::SCRATCH[10],
        2,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[12],
        abi::SCRATCH[9],
        abi::SCRATCH[12],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[11], abi::SCRATCH[12], 0));
    builder.emit(abi::add_immediate(abi::SCRATCH[10], abi::SCRATCH[10], 1));
    builder.emit(abi::branch(&copy_head));
    builder.emit(abi::label(&copy_done));

    // --- the glyph's own item block ----------------------------------------------
    // quad is 16.16 like every other kind's; shape.x/.y are WHOLE pixels, because the
    // shader indexes the bitmap with them and a fixed-point origin would have to be
    // converted back — losing the exactness that makes the lookup a lookup.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_x,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_y,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_SHAPE,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_SHAPE + 4,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[0],
        abi::SCRATCH[2],
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
    ));
    // The per-glyph quad narrows the item's quad to this one glyph's box, so a run of
    // twenty glyphs rasterises twenty small quads instead of twenty copies of the whole
    // run's box. That narrowing is only valid UNTRANSFORMED: the box is in shape space,
    // and under a transform the glyph's pixels are somewhere else entirely — the GPU
    // would rasterise a region the glyph no longer occupies and draw nothing.
    //
    // Transformed, the item's quad is left as `emit_item_block` wrote it, which is the
    // whole run's transformed hull (`__canvas_boundsHeader` maps the four corners
    // forward). Correct, and the cost is stated rather than hidden: every glyph in a
    // transformed run rasterises the run's hull, so a long transformed string is
    // O(glyphs × hull) fragments. Narrowing it would mean forward-mapping four corners
    // in emitted machine code, in both backends, to save fragments on a case that is
    // not the common one.
    {
        let keep_hull = builder.label("vk_glyph_hull");
        builder.emit(abi::load_u32(
            abi::SCRATCH[6],
            abi::stack_pointer(),
            at.item + ITEM_OFFSET_TRANSFORM + 24,
        ));
        builder.emit(abi::compare_immediate(abi::SCRATCH[6], "0"));
        builder.emit(abi::branch_ne(&keep_hull));
        for (register, word) in [
            (abi::SCRATCH[0], 0usize),
            (abi::SCRATCH[1], 1),
            (abi::SCRATCH[4], 2),
            (abi::SCRATCH[5], 3),
        ] {
            builder.emit(abi::shift_left_immediate(abi::SCRATCH[6], register, 16));
            builder.emit(abi::store_u32(
                abi::SCRATCH[6],
                abi::stack_pointer(),
                at.item + ITEM_OFFSET_QUAD + word * 4,
            ));
        }
        builder.emit(abi::label(&keep_hull));
    }
    // misc.w = width, arc.x = height, arc.z = the sample offset into the region.
    builder.emit(abi::store_u32(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));

    // Advance the cursor before the calls: `w * h` is in a scratch register the calls
    // are free to clobber.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));

    // --- publish and draw ------------------------------------------------------------
    // This glyph's block goes into the frame's item buffer like any other quad's, and
    // the draw names it through `firstInstance`. The index is parked *before* the
    // publish, because publishing advances the cursor past it.
    builder.emit(abi::label(&publish));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.instance,
    ));
    emit_item_publish(builder, at.state, at.item, at.item_cursor, &next);
    builder.emit(abi::branch(&next));

    // A glyph that draws nothing — a space, an evicted cache entry, a bitmap past the
    // frame's glyph region — still publishes ONE block, with width and height 0, so the
    // shader's bounds test in `glyphCoverage` answers zero coverage everywhere (bug-484).
    //
    // Skipping the publish desynchronised the item buffer from the draw list:
    // `__canvas_blockInstances` counts a text run as its full glyph count, so every draw
    // entry after a run containing a space named blocks one position on — the next items
    // drew under their neighbours' pipelines and group offsets, and the last read a block
    // never written. Found on Metal by bug-484's picture scene ("Pictures 01"); the
    // Vulkan gate's only label, "AAAA", has no space. The quad is left as it is (the
    // previous glyph's box, or the run's hull under a transform): a transparent source
    // changes nothing under any blend pipeline.
    builder.emit(abi::label(&blank));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    builder.emit(abi::branch(&publish));

    // plan-116-H: this function now PUBLISHES and does not draw. Its `vkCmdDraw` moved
    // to `emit_draw_list_pass`, with every other draw, because the draw list is in scene
    // order and a group's blocks are shared — neither of which a draw issued from the
    // publish walk can honour (H9).
    //
    // Leaving the draw here was measured, not theorised: the glyphs were then drawn
    // twice, once from here with the push constant still undefined for that command, so
    // the vertex stage offset them by whatever the range held. The Vulkan harness
    // reported the render disagreeing with the oracle on 35.1% of pixels, worst=255,
    // while every draw-list assertion passed — the list was right and the command
    // stream had extra draws in it.
    //
    // The block is still published one per glyph quad at the shared cursor, which is
    // what lets the draw list count a text run as N contiguous instances and issue it
    // as a single instanced call.

    builder.emit(abi::label(&next));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_index,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_index,
    ));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
}
