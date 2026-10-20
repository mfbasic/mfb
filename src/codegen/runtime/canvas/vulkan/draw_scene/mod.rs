//! plan-176-V: the graphics thread's frame emitter (`emit_vulkan_draw_scene`) as consecutive phases
//! over one frame: the slots it shares (`DrawSceneSlots`), the argument park and the frame setup
//! here, the record and item walk in `record.rs`, the finish in `finish.rs`. The shared constants,
//! layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

mod finish;
mod record;

use finish::emit_finish_frame;
use record::{emit_item_walk, emit_record_begin};

/// Every frame label and stack slot `emit_vulkan_draw_scene` shares across its phases. Built
/// once by `allocate`, in the original allocation order (byte-identity).
struct DrawSceneSlots {
    done: String,
    unavailable: String,
    item_head: String,
    item_done: String,
    item_next: String,
    text_item: String,
    swizzle_head: String,
    swizzle_done: String,
    off_state: usize,
    off_handle: usize,
    off_fn: usize,
    off_out: usize,
    off_surface: usize,
    off_width: usize,
    off_height: usize,
    off_geometry: usize,
    off_offsets: usize,
    off_draws: usize,
    off_count: usize,
    off_index: usize,
    off_item: usize,
    off_begin: usize,
    off_clear: usize,
    off_pass_begin: usize,
    off_viewport: usize,
    off_scissor: usize,
    off_copy: usize,
    off_submit: usize,
    off_cmd_handle: usize,
    off_desc_set_handle: usize,
    off_edge_cursor: usize,
    off_grad_cursor: usize,
    off_dash_cursor: usize,
    off_item_cursor: usize,
    off_run_start: usize,
    off_run_count: usize,
    off_bound_mode: usize,
    off_item_mode: usize,
    off_pipe_fn: usize,
    off_saved_stroke: usize,
    off_header: usize,
    off_glyph_meta: usize,
    off_glyph_cov: usize,
    off_glyph_cursor: usize,
    off_pic_count: usize,
    off_pic_slot: usize,
    bands: BandSlots,
    off_glyph_index: usize,
    off_glyph_count: usize,
    off_glyph_w: usize,
    off_glyph_h: usize,
    off_glyph_x: usize,
    off_glyph_y: usize,
    off_glyph_instance: usize,
    off_draw_fn: usize,
    swapchain: Option<present::PresentSlots>,
    off_draw_entries: usize,
    off_pass: usize,
}

impl DrawSceneSlots {
    fn allocate(builder: &mut CodeBuilder, present_mode: bool) -> DrawSceneSlots {
        let done = builder.label("vk_draw_done");
        let unavailable = builder.label("vk_draw_unavailable");
        let item_head = builder.label("vk_draw_item_head");
        let item_done = builder.label("vk_draw_item_done");
        let item_next = builder.label("vk_draw_item_next");
        let text_item = builder.label("vk_draw_text_item");
        let swizzle_head = builder.label("vk_draw_swizzle_head");
        let swizzle_done = builder.label("vk_draw_swizzle_done");

        let off_state = builder.allocate_stack_object("vk_state", 8);
        let off_handle = builder.allocate_stack_object("vk_handle", 8);
        let off_fn = builder.allocate_stack_object("vk_fn", 8);
        let off_out = builder.allocate_stack_object("vk_out", 8);
        let off_surface = builder.allocate_stack_object("vk_surface", 8);
        let off_width = builder.allocate_stack_object("vk_width", 8);
        let off_height = builder.allocate_stack_object("vk_height", 8);
        let off_geometry = builder.allocate_stack_object("vk_geometry", 8);
        let off_offsets = builder.allocate_stack_object("vk_offsets", 8);
        // plan-116-H: the draw list, parked beside the block list. Four integers per entry.
        let off_draws = builder.allocate_stack_object("vk_draws", 8);
        let off_count = builder.allocate_stack_object("vk_draw_count", 8);
        let off_index = builder.allocate_stack_object("vk_draw_index", 8);
        let off_item = builder.allocate_stack_object("vk_item", ITEM_BLOCK_SIZE);
        let off_begin = builder.allocate_stack_object("vk_cmd_begin", CMD_BEGIN_INFO_SIZE);
        let off_clear = builder.allocate_stack_object("vk_clear", CLEAR_VALUE_SIZE);
        let off_pass_begin = builder.allocate_stack_object("vk_pass_begin", PASS_BEGIN_INFO_SIZE);
        let off_viewport = builder.allocate_stack_object("vk_viewport", VIEWPORT_SIZE);
        let off_scissor = builder.allocate_stack_object("vk_scissor", RECT_SIZE);
        let off_copy = builder.allocate_stack_object("vk_copy", COPY_SIZE);
        let off_submit = builder.allocate_stack_object("vk_submit", SUBMIT_INFO_SIZE);
        let off_cmd_handle = builder.allocate_stack_object("vk_cmd_handle", 8);
        let off_desc_set_handle = builder.allocate_stack_object("vk_desc_set_handle", 8);
        let off_edge_cursor = builder.allocate_stack_object("vk_edge_cursor", 8);
        // The frame's gradient-stop cursor — the third region's twin of the edge one,
        // counted in STOPS rather than words so the arithmetic below matches the shader's.
        let off_grad_cursor = builder.allocate_stack_object("vk_grad_cursor", 8);
        // plan-163-B: the frame's dash-region cursor, in words.
        let off_dash_cursor = builder.allocate_stack_object("vk_dash_cursor", 8);
        // The frame's item-buffer cursor, and the base of the instanced run currently being
        // accumulated. `run_count` is scratch for the flush, which has to compute
        // `cursor - run_start` somewhere the argument staging cannot clobber.
        let off_item_cursor = builder.allocate_stack_object("vk_item_cursor", 8);
        let off_run_start = builder.allocate_stack_object("vk_run_start", 8);
        let off_run_count = builder.allocate_stack_object("vk_run_count", 8);
        // The blend mode currently bound, and this item's. A mode change ends the
        // instanced run exactly as a glyph run does, which is what keeps paint order
        // exact: items batch only while they are adjacent AND share a mode (plan-116-B).
        let off_bound_mode = builder.allocate_stack_object("vk_bound_mode", 8);
        let off_item_mode = builder.allocate_stack_object("vk_item_mode", 8);
        let off_pipe_fn = builder.allocate_stack_object("vk_pipe_fn", 8);
        let off_saved_stroke = builder.allocate_stack_object("vk_saved_stroke", 8);
        let off_header = builder.allocate_stack_object("vk_header", 8);
        // The glyph cache, and the frame's running cursor into the buffer's glyph region.
        let off_glyph_meta = builder.allocate_stack_object("vk_glyph_meta", 8);
        let off_glyph_cov = builder.allocate_stack_object("vk_glyph_cov", 8);
        let off_glyph_cursor = builder.allocate_stack_object("vk_glyph_cursor", 8);
        // bug-688: the frame's picture-table record count and a lookup's parked index slot.
        let off_pic_count = builder.allocate_stack_object("vk_pic_count", 8);
        let off_pic_slot = builder.allocate_stack_object("vk_pic_slot", 8);
        // bug-688: the band index's region cursor and one polygon's working state.
        let bands = BandSlots::allocate(builder, off_state, off_item, off_header);
        let off_glyph_index = builder.allocate_stack_object("vk_glyph_index", 8);
        let off_glyph_count = builder.allocate_stack_object("vk_glyph_count", 8);
        let off_glyph_w = builder.allocate_stack_object("vk_glyph_w", 8);
        let off_glyph_h = builder.allocate_stack_object("vk_glyph_h", 8);
        let off_glyph_x = builder.allocate_stack_object("vk_glyph_x", 8);
        let off_glyph_y = builder.allocate_stack_object("vk_glyph_y", 8);
        let off_glyph_instance = builder.allocate_stack_object("vk_glyph_instance", 8);
        // `vkCmdDraw`, resolved once and kept, because the glyph path calls it per glyph
        // rather than per item and a `dlsym` per glyph would be a string comparison in the
        // inner loop of every string on screen. `vkCmdPushConstants` used to be resolved
        // here beside it and is gone: the item block travels in a buffer now, so nothing
        // pushes a constant.
        let off_draw_fn = builder.allocate_stack_object("vk_draw_fn", 8);
        let swapchain = present_mode.then(|| present::PresentSlots::allocate(builder));
        let off_draw_entries = builder.allocate_stack_object("vk_draw_entries", 8);
        let off_pass = builder.allocate_stack_object("vk_pass_word", 8);
        DrawSceneSlots {
            done,
            unavailable,
            item_head,
            item_done,
            item_next,
            text_item,
            swizzle_head,
            swizzle_done,
            off_state,
            off_handle,
            off_fn,
            off_out,
            off_surface,
            off_width,
            off_height,
            off_geometry,
            off_offsets,
            off_draws,
            off_count,
            off_index,
            off_item,
            off_begin,
            off_clear,
            off_pass_begin,
            off_viewport,
            off_scissor,
            off_copy,
            off_submit,
            off_cmd_handle,
            off_desc_set_handle,
            off_edge_cursor,
            off_grad_cursor,
            off_dash_cursor,
            off_item_cursor,
            off_run_start,
            off_run_count,
            off_bound_mode,
            off_item_mode,
            off_pipe_fn,
            off_saved_stroke,
            off_header,
            off_glyph_meta,
            off_glyph_cov,
            off_glyph_cursor,
            off_pic_count,
            off_pic_slot,
            off_glyph_index,
            off_glyph_count,
            off_glyph_w,
            off_glyph_h,
            off_glyph_x,
            off_glyph_y,
            off_glyph_instance,
            off_draw_fn,
            bands,
            swapchain,
            off_draw_entries,
            off_pass,
        }
    }
}

pub(crate) fn emit_vulkan_draw_scene(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    // `None` is `canvas::vulkanPresentScene` (bug-688 V6): no surface, the frame goes
    // to the window's swapchain, and the answer (presented or not) is left in
    // `RESULT_VALUE_REGISTER`.
    surface: Option<&Operand>,
    width: &Operand,
    height: &Operand,
    geometry: &Operand,
    offsets: &Operand,
    glyph_meta: &Operand,
    glyph_coverage: &Operand,
    draws: &Operand,
) -> Result<(), String> {
    let present_mode = surface.is_none();
    if !has_vulkan_backend(platform) || (present_mode && !present::has_present_path(platform)) {
        if present_mode {
            builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        }
        return Ok(());
    }
    let s = DrawSceneSlots::allocate(builder, present_mode);
    emit_park_arguments(
        builder,
        &s,
        surface,
        width,
        height,
        geometry,
        offsets,
        glyph_meta,
        glyph_coverage,
        draws,
    );
    let off_render_pass = emit_frame_setup(builder, platform, platform_imports, &s, present_mode)?;
    emit_record_begin(builder, platform, platform_imports, &s, off_render_pass)?;
    emit_item_walk(builder, &s);
    emit_finish_frame(builder, platform, platform_imports, &s)
}

/// `emit_vulkan_draw_scene` phase 1: park the arguments in their frame slots before anything
/// calls, and the draw-entry count and pass word with them.
#[allow(clippy::too_many_arguments)]
fn emit_park_arguments(
    builder: &mut CodeBuilder,
    s: &DrawSceneSlots,
    surface: Option<&Operand>,
    width: &Operand,
    height: &Operand,
    geometry: &Operand,
    offsets: &Operand,
    glyph_meta: &Operand,
    glyph_coverage: &Operand,
    draws: &Operand,
) {
    // Park the arguments before anything calls.
    if let Some(surface) = surface {
        builder.emit(abi::add_immediate(
            abi::SCRATCH[0],
            surface.clone(),
            COLLECTION_HEADER_SIZE,
        ));
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_surface,
        ));
    }
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        geometry.clone(),
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_geometry,
    ));
    // plan-116-H: `draws` is read FIRST, before any other argument is staged.
    //
    // It is the EIGHTH parameter, which MFBASIC's convention places in `rbp`
    // (bug-296), and the staging below writes `SCRATCH` registers that alias the C
    // argument bank on x86-64. Reading it after that staging returns whatever the bank
    // now holds — a short entry count, so the walk stops early and the LAST items of the
    // scene are never drawn. Measured: the three gradient items, which are last in the
    // harness scene, rendered as their black `fill` while everything before them was
    // pixel-correct, in every variant of the draw list tried.
    //
    // This is `.ai/arch-abi.md`'s "stage ABI args via temporaries" rule, and the reason
    // it bites here rather than earlier is that no previous argument was the eighth.
    // plan-116-H: the draw list's data pointer, parked the same way and for the same
    // reason — an argument register does not survive the calls below.
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        draws.clone(),
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_draws,
    ));
    // The ENTRY count, which is the element count divided by four — each entry is
    // `(base, count, dx, dy)`. Storing the element count instead makes the walk below
    // run four times too long, reading `base`/`count` out of whatever follows the list:
    // a garbage `instanceCount` that hangs the GPU rather than failing, which is what
    // the first version of this did (timeout on box 2228, no output).
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        draws.clone(),
        COLLECTION_OFFSET_COUNT as usize,
    ));
    builder.emit(abi::shift_right_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        CANVAS_DRAW_ENTRY_COUNT_SHIFT as u8,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_draw_entries,
    ));
    // bug-765: the pass word, off the list's first entry (`CANVAS_DRAW_PASS_WORD`): 0
    // for a frame of one pass, which is also what a list with no entry is -- and the
    // entry count is what `SCRATCH[0]` holds here, so a list with none stores it.
    //
    // `SCRATCH[0]` and nothing else: the arguments below are not parked yet, and on
    // x86-64 the other scratch registers ARE argument registers (`.ai/arch-abi.md`). A
    // first cut used `SCRATCH[1]` and overwrote the glyph metadata pointer, so every
    // frame with a `Text` faulted on Windows and only there.
    {
        let no_entry = builder.label("vk_no_pass_word");
        builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
        builder.emit(abi::branch_eq(&no_entry));
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            draws.clone(),
            COLLECTION_HEADER_SIZE + CANVAS_DRAW_PASS_WORD * 8,
        ));
        builder.emit(abi::label(&no_entry));
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_pass,
        ));
    }

    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        offsets.clone(),
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_count,
    ));
    for (source, slot) in [
        (glyph_meta, s.off_glyph_meta),
        (glyph_coverage, s.off_glyph_cov),
    ] {
        builder.emit(abi::add_immediate(
            abi::SCRATCH[0],
            source.clone(),
            COLLECTION_HEADER_SIZE,
        ));
        builder.emit(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
    }
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        offsets.clone(),
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_offsets,
    ));
    builder.emit(abi::store_u64(
        width.clone(),
        abi::stack_pointer(),
        s.off_width,
    ));
    builder.emit(abi::store_u64(
        height.clone(),
        abi::stack_pointer(),
        s.off_height,
    ));
}

/// `emit_vulkan_draw_scene` phase 2: begin the present, resume a later pass, check the renderer
/// is ready, build the render target and choose this pass's render pass.
fn emit_frame_setup(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    s: &DrawSceneSlots,
    present_mode: bool,
) -> Result<usize, String> {
    if let Some(at) = &s.swapchain {
        present::emit_present_begin(builder, at);
    }

    // The renderer must be ready. `canvas::vulkanReady` is the branch's own
    // condition, so this is a guard against being called out of order, not a
    // fallback: a frame that got here without a pipeline renders nothing and leaves
    // the cleared surface the software path would have produced.
    state_base_into(builder, s.off_state);
    // bug-765: a later pass of a frame takes up the swapchain image its first pass
    // acquired, before anything below can leave: every exit that gives the frame up
    // must find the image to give up with it (`emit_present_abandon`).
    if let Some(at) = &s.swapchain {
        let first = builder.label("vk_sc_not_resumed");
        emit_branch_on_pass(
            builder,
            s.off_pass,
            CANVAS_DRAW_PASS_CONTINUES,
            false,
            &first,
        );
        present::emit_present_resume(builder, s.off_state, at);
        builder.emit(abi::label(&first));
    }
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_READY,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "1"));
    builder.emit(abi::branch_ne(&s.unavailable));
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_LIB,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_handle,
    ));

    emit_vulkan_target(
        builder,
        platform,
        platform_imports,
        s.off_handle,
        s.off_fn,
        s.off_state,
        s.off_out,
        s.off_width,
        s.off_height,
        &s.unavailable,
    )?;
    // bug-765: the render pass this list is recorded under. A frame's first pass (and a
    // frame of one) clears the target: `RENDER_PASS`, or `PRESENT_PASS` on a swapchain
    // image. A later pass loads it, under the twin that differs only in its load
    // operation and layouts, made here the first time a frame needs it.
    let off_render_pass = builder.allocate_stack_object("vk_render_pass", 8);
    let (clear_slot, load_slot, load_layout) = if present_mode {
        (
            GRAPHICS_OFFSET_VULKAN_PRESENT_PASS,
            GRAPHICS_OFFSET_VULKAN_PRESENT_LOAD_PASS,
            present::IMAGE_LAYOUT_PRESENT_SRC,
        )
    } else {
        (
            GRAPHICS_OFFSET_VULKAN_RENDER_PASS,
            GRAPHICS_OFFSET_VULKAN_LOAD_PASS,
            IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
        )
    };
    {
        let first = builder.label("vk_first_pass");
        let chosen = builder.label("vk_pass_chosen");
        emit_branch_on_pass(
            builder,
            s.off_pass,
            CANVAS_DRAW_PASS_CONTINUES,
            false,
            &first,
        );
        if let Some(at) = &s.swapchain {
            // Nothing held is a frame whose first pass did not acquire: give it up.
            present::emit_require_acquired(builder, at, &s.unavailable);
        }
        emit_load_render_pass(
            builder,
            platform,
            platform_imports,
            s.off_handle,
            s.off_fn,
            s.off_state,
            s.off_out,
            load_slot,
            load_layout,
            &s.unavailable,
        )?;
        emit_state_load(builder, s.off_state, load_slot, abi::SCRATCH[0]);
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_render_pass,
        ));
        builder.emit(abi::branch(&chosen));
        builder.emit(abi::label(&first));
        if let Some(at) = &s.swapchain {
            present::emit_ensure_swapchain(
                builder,
                platform,
                platform_imports,
                s.off_handle,
                s.off_fn,
                s.off_state,
                s.off_out,
                s.off_width,
                s.off_height,
                at,
                &s.unavailable,
            )?;
        }
        emit_state_load(builder, s.off_state, clear_slot, abi::SCRATCH[0]);
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_render_pass,
        ));
        builder.emit(abi::label(&chosen));
    }
    Ok(off_render_pass)
}
