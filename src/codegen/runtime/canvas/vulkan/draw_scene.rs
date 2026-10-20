//! plan-176-V: the graphics thread's frame emitter (`emit_vulkan_draw_scene`). The shared
//! constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

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
            off_surface,
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
        off_geometry,
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
        off_draws,
    ));
    // The ENTRY count, which is the element count divided by four — each entry is
    // `(base, count, dx, dy)`. Storing the element count instead makes the walk below
    // run four times too long, reading `base`/`count` out of whatever follows the list:
    // a garbage `instanceCount` that hangs the GPU rather than failing, which is what
    // the first version of this did (timeout on box 2228, no output).
    let off_draw_entries = builder.allocate_stack_object("vk_draw_entries", 8);
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
        off_draw_entries,
    ));
    // bug-765: the pass word, off the list's first entry (`CANVAS_DRAW_PASS_WORD`): 0
    // for a frame of one pass, which is also what a list with no entry is -- and the
    // entry count is what `SCRATCH[0]` holds here, so a list with none stores it.
    //
    // `SCRATCH[0]` and nothing else: the arguments below are not parked yet, and on
    // x86-64 the other scratch registers ARE argument registers (`.ai/arch-abi.md`). A
    // first cut used `SCRATCH[1]` and overwrote the glyph metadata pointer, so every
    // frame with a `Text` faulted on Windows and only there.
    let off_pass = builder.allocate_stack_object("vk_pass_word", 8);
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
            off_pass,
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
        off_count,
    ));
    for (source, slot) in [
        (glyph_meta, off_glyph_meta),
        (glyph_coverage, off_glyph_cov),
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
        off_offsets,
    ));
    builder.emit(abi::store_u64(
        width.clone(),
        abi::stack_pointer(),
        off_width,
    ));
    builder.emit(abi::store_u64(
        height.clone(),
        abi::stack_pointer(),
        off_height,
    ));

    if let Some(at) = &swapchain {
        present::emit_present_begin(builder, at);
    }

    // The renderer must be ready. `canvas::vulkanReady` is the branch's own
    // condition, so this is a guard against being called out of order, not a
    // fallback: a frame that got here without a pipeline renders nothing and leaves
    // the cleared surface the software path would have produced.
    state_base_into(builder, off_state);
    // bug-765: a later pass of a frame takes up the swapchain image its first pass
    // acquired, before anything below can leave: every exit that gives the frame up
    // must find the image to give up with it (`emit_present_abandon`).
    if let Some(at) = &swapchain {
        let first = builder.label("vk_sc_not_resumed");
        emit_branch_on_pass(builder, off_pass, CANVAS_DRAW_PASS_CONTINUES, false, &first);
        present::emit_present_resume(builder, off_state, at);
        builder.emit(abi::label(&first));
    }
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READY,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "1"));
    builder.emit(abi::branch_ne(&unavailable));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_LIB,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_handle,
    ));

    emit_vulkan_target(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_out,
        off_width,
        off_height,
        &unavailable,
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
        emit_branch_on_pass(builder, off_pass, CANVAS_DRAW_PASS_CONTINUES, false, &first);
        if let Some(at) = &swapchain {
            // Nothing held is a frame whose first pass did not acquire: give it up.
            present::emit_require_acquired(builder, at, &unavailable);
        }
        emit_load_render_pass(
            builder,
            platform,
            platform_imports,
            off_handle,
            off_fn,
            off_state,
            off_out,
            load_slot,
            load_layout,
            &unavailable,
        )?;
        emit_state_load(builder, off_state, load_slot, abi::SCRATCH[0]);
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_render_pass,
        ));
        builder.emit(abi::branch(&chosen));
        builder.emit(abi::label(&first));
        if let Some(at) = &swapchain {
            present::emit_ensure_swapchain(
                builder,
                platform,
                platform_imports,
                off_handle,
                off_fn,
                off_state,
                off_out,
                off_width,
                off_height,
                at,
                &unavailable,
            )?;
        }
        emit_state_load(builder, off_state, clear_slot, abi::SCRATCH[0]);
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_render_pass,
        ));
        builder.emit(abi::label(&chosen));
    }

    // --- record ------------------------------------------------------------------
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_struct(
        builder,
        off_begin,
        CMD_BEGIN_INFO_SIZE,
        &[
            (0, Field::U32(ST_COMMAND_BUFFER_BEGIN_INFO)),
            (CMD_BEGIN_FLAGS, Field::U32(COMMAND_BUFFER_ONE_TIME_SUBMIT)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkBeginCommandBuffer",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_begin,
    ));
    emit_call_fn(builder, off_fn);

    // Opaque black, matching `canvas::newSurface`, so both backends start from the
    // same pixels without either naming the colour twice.
    emit_struct(
        builder,
        off_clear,
        CLEAR_VALUE_SIZE,
        &[(CLEAR_ALPHA, Field::U32(FLOAT_ONE_BITS_F32))],
    );
    emit_struct(
        builder,
        off_pass_begin,
        PASS_BEGIN_INFO_SIZE,
        &[
            (0, Field::U32(ST_RENDER_PASS_BEGIN_INFO)),
            (PASS_BEGIN_CLEAR_COUNT, Field::U32("1")),
            (PASS_BEGIN_CLEARS, Field::Addr(off_clear)),
        ],
    );
    // The swapchain image's pass differs only in its final layout, and a later pass's
    // only in its load operation and layouts, so each is compatible with the offscreen
    // pass every pipeline was built against.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_render_pass,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_pass_begin + PASS_BEGIN_RENDER_PASS,
    ));
    match &swapchain {
        Some(at) => builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.framebuffer,
        )),
        None => emit_state_load(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER,
            abi::SCRATCH[0],
        ),
    }
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_pass_begin + PASS_BEGIN_FRAMEBUFFER,
    ));
    for (field, parked) in [
        (PASS_BEGIN_AREA_WIDTH, off_width),
        (PASS_BEGIN_AREA_HEIGHT, off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_pass_begin + field,
        ));
    }
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBeginRenderPass",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_pass_begin,
    ));
    builder.emit(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        SUBPASS_CONTENTS_INLINE,
    ));
    emit_call_fn(builder, off_fn);

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBindPipeline",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        PIPELINE_BIND_POINT_GRAPHICS,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PIPELINE,
        abi::c_arg(2),
    );
    emit_call_fn(builder, off_fn);

    // The polygon edge buffer, bound once for the whole frame. Every polygon in the
    // scene reads from this one buffer at its own offset (`ITEM_ARC_EDGE_BASE`),
    // which is why it can be bound here rather than re-bound per item: a Vulkan
    // command buffer is *recorded* now and *executed* later, so a per-item rebind of
    // one shared buffer would not give each item its own view of it — every polygon
    // would see whatever the last one wrote.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBindDescriptorSets",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        PIPELINE_BIND_POINT_GRAPHICS,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
        abi::c_arg(2),
    );
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // firstSet
    emit_int_arg(builder, platform, 4, "1"); // setCount
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DESC_SET,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_desc_set_handle,
    ));
    emit_addr_arg(builder, platform, 5, off_desc_set_handle);
    emit_int_arg_zero(builder, platform, 6); // dynamicOffsetCount
    emit_int_arg_zero(builder, platform, 7); // pDynamicOffsets
    emit_call_fn(builder, off_fn);

    // Viewport and scissor are dynamic state, so they are set per frame — which is
    // exactly what lets a resize reuse the pipeline.
    emit_struct(
        builder,
        off_viewport,
        VIEWPORT_SIZE,
        &[(VIEWPORT_MAX_DEPTH, Field::U32(FLOAT_ONE_BITS_F32))],
    );
    for (field, parked) in [(VIEWPORT_WIDTH, off_width), (VIEWPORT_HEIGHT, off_height)] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        emit_store_f32_from_integer(builder, abi::SCRATCH[0], off_viewport + field);
    }
    emit_struct(builder, off_scissor, RECT_SIZE, &[]);
    for (field, parked) in [(RECT_WIDTH, off_width), (RECT_HEIGHT, off_height)] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_scissor + field,
        ));
    }
    for (name, slot) in [
        ("vkCmdSetViewport", off_viewport),
        ("vkCmdSetScissor", off_scissor),
    ] {
        emit_dlsym(
            builder,
            platform,
            platform_imports,
            name,
            off_handle,
            off_fn,
            &unavailable,
        )?;
        builder.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            off_cmd_handle,
        ));
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
        builder.emit(abi::add_immediate(
            abi::c_arg(3),
            abi::stack_pointer(),
            slot,
        ));
        emit_call_fn(builder, off_fn);
    }

    // `vkCmdDraw`, resolved once. Every draw in the frame goes through this one slot
    // now — the run flushes and the per-glyph draws alike — so the shape path no longer
    // re-resolves anything per item.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdDraw",
        off_handle,
        off_draw_fn,
        &unavailable,
    )?;
    // `vkCmdBindPipeline`, resolved once for the same reason: it is now called per RUN
    // rather than once per frame, and a scene that alternates blend modes would put a
    // `dlsym` between every pair of items.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBindPipeline",
        off_handle,
        off_pipe_fn,
        &unavailable,
    )?;

    // --- one quad per item --------------------------------------------------------
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    // The frame's running edge cursor: each polygon appends its edges here and
    // records where they started in its own item block.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_glyph_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_grad_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_dash_cursor,
    ));
    // bug-688: an empty picture table. Nothing in the table itself is cleared -- a slot
    // is live only below this count (`emit_picture_lookup`).
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_pic_count,
    ));
    // bug-688: and an empty band region.
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        bands.cursor,
    ));
    // The item-buffer cursor and the current run's base, both starting at quad zero.
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_run_start,
    ));
    // `Normal` is what the once-per-frame bind above left bound, so the frame starts
    // knowing that. An all-`Normal` scene therefore issues exactly the one
    // `vkCmdBindPipeline` it always did.
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_bound_mode,
    ));
    builder.emit(abi::label(&item_head));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_ge(&item_done));

    // header = geometry + offsets[i] * 8
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_offsets,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));
    builder.emit(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_geometry,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_header,
    ));
    // --- the blend mode, before the kind fork so a glyph run takes it too -----------
    // A mode change ends the instanced run and binds that mode's pipeline. Ending the
    // run is what preserves paint order: the items already published draw under the
    // pipeline they were recorded with, and only then does the next one take over.
    // Batching is therefore ADJACENT-run only — a scene that alternates modes issues
    // more binds and one that groups them issues few, but neither reorders anything.
    {
        let same_mode = builder.label("vk_same_mode");
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[0],
            HEADER_BLEND * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item_mode,
        ));
        builder.emit(abi::load_u64(
            abi::SCRATCH[2],
            abi::stack_pointer(),
            off_bound_mode,
        ));
        builder.emit(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[2]));
        builder.emit(abi::branch_eq(&same_mode));

        // plan-116-H: no flush here any more. The draw list already ends a run wherever
        // the blend mode changes (H8), so this site's only remaining job is to bind the
        // pipeline the following blocks will be drawn with; the draws themselves are
        // issued by `emit_draw_list_pass` once every block is published (H9).
        //
        // handle = *(state + …_PIPELINE_MODES + mode * 8). Contiguous and 0-based, so
        // this is a shift and an add rather than a four-way branch.
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_item_mode,
        ));
        builder.emit(abi::shift_left_immediate(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            3,
        ));
        builder.emit(abi::load_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_state,
        ));
        builder.emit(abi::add_registers(
            abi::SCRATCH[0],
            abi::SCRATCH[1],
            abi::SCRATCH[0],
        ));
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES,
        ));
        // Parked, then staged from memory: `load_selector`-free though this is, the
        // argument staging below still runs through the C bank that aliases SCRATCH on
        // x86-64 (`.ai/arch-abi.md`).
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_run_count,
        ));
        builder.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            off_cmd_handle,
        ));
        builder.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            PIPELINE_BIND_POINT_GRAPHICS,
        ));
        builder.emit(abi::load_u64(
            abi::c_arg(2),
            abi::stack_pointer(),
            off_run_count,
        ));
        emit_call_fn(builder, off_pipe_fn);

        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_item_mode,
        ));
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_bound_mode,
        ));
        builder.emit(abi::label(&same_mode));
        // The header address was in SCRATCH[0] on entry and the block above used it;
        // restore it before the kind fork reads it.
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_header,
        ));
    }

    // A glyph run is not one draw, so it forks before the item block is built: the
    // block a glyph needs describes the *glyph*, not the run.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
        HEADER_KIND * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_TEXT));
    builder.emit(abi::branch_eq(&text_item));

    emit_item_block(builder, off_item, off_width, off_height);
    emit_edge_upload(
        builder,
        off_state,
        off_item,
        off_header,
        off_edge_cursor,
        &bands,
    );
    // plan-163-B: after the item's own edges, so its join records follow them; then the
    // dash words, after the join upload has zeroed their block words.
    emit_join_upload(builder, off_state, off_item, off_header, off_edge_cursor);
    emit_dash_upload(builder, off_state, off_item, off_header, off_dash_cursor);
    // plan-163-C: the clip's words and a path clip's edges, after the join records.
    emit_clip_upload(
        builder,
        off_state,
        off_item,
        off_header,
        off_edge_cursor,
        &bands,
    );
    emit_gradient_upload(builder, off_state, off_item, off_header, off_grad_cursor);
    // bug-484: after the edge upload (which zeroes the slice fields for non-polygons)
    // and before the split, so both records of a split picture name its texels.
    emit_picture_upload(
        builder,
        PictureSlots {
            state: off_state,
            item: off_item,
            header: off_header,
            glyph_cursor: off_glyph_cursor,
            pic_count: off_pic_count,
            pic_slot: off_pic_slot,
        },
    );
    // Published, not drawn. The draw happens at the end of the run this item joins —
    // which is what makes consecutive shapes one instanced `vkCmdDraw` instead of N.
    emit_split_or_publish(
        builder,
        off_state,
        off_item,
        off_item_cursor,
        off_item_mode,
        off_saved_stroke,
        &item_next,
    );
    builder.emit(abi::branch(&item_next));

    // plan-116-H: no flush here either. Ordering used to be maintained by issuing the
    // accumulated run before the glyphs; it is now maintained by the draw list being in
    // scene order and the second pass walking it in that order. A `Text` item is its own
    // entry (H8), so nothing can merge across it.
    builder.emit(abi::label(&text_item));
    emit_glyph_publish(
        builder,
        GlyphPublishSlots {
            state: off_state,
            item: off_item,
            header: off_header,
            width: off_width,
            height: off_height,
            item_cursor: off_item_cursor,
            instance: off_glyph_instance,
            glyph_meta: off_glyph_meta,
            glyph_cov: off_glyph_cov,
            glyph_cursor: off_glyph_cursor,
            glyph_index: off_glyph_index,
            glyph_count: off_glyph_count,
            glyph_w: off_glyph_w,
            glyph_h: off_glyph_h,
            glyph_x: off_glyph_x,
            glyph_y: off_glyph_y,
        },
    );
    // The glyphs consumed item-buffer slots of their own, so the next run of shapes
    // begins after them — not where the flush above left the base. Without this the
    // scene's trailing shapes are drawn as one run that *starts at the first glyph*,
    // so every glyph quad is drawn a second time. That is invisible for an opaque
    // glyph — compositing an opaque square over itself is idempotent — which is
    // precisely why `scripts/test-canvas-vulkan.sh` draws its label translucent.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_run_start,
    ));

    builder.emit(abi::label(&item_next));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::branch(&item_head));
    builder.emit(abi::label(&item_done));

    // plan-116-H: every block is now published, so the second pass issues every draw.
    // This replaces the trailing flush *and* the two removed above — one walk over the
    // draw list, in scene order, one instanced call per entry with that entry's group
    // offset pushed first.
    emit_draw_list_pass(
        builder,
        platform,
        platform_imports,
        off_state,
        off_handle,
        off_cmd_handle,
        off_draw_fn,
        off_draws,
        off_draw_entries,
        &unavailable,
    )?;

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdEndRenderPass",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_call_fn(builder, off_fn);

    if let Some(at) = &swapchain {
        present::emit_submit_and_present(
            builder,
            platform,
            platform_imports,
            off_handle,
            off_fn,
            off_state,
            off_cmd_handle,
            off_pass,
            at,
            &unavailable,
        )?;
        builder.emit(abi::branch(&done));
        builder.emit(abi::label(&unavailable));
        present::emit_present_abandon(
            builder,
            platform,
            platform_imports,
            off_handle,
            off_fn,
            off_state,
            at,
        )?;
        builder.emit(abi::label(&done));
        builder.emit(abi::load_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            at.presented,
        ));
        return Ok(());
    }

    // --- copy the image out -------------------------------------------------------
    // bug-765: by the frame's LAST pass only. A pass with more to come leaves the image
    // where it is; it is still submitted and waited for below, because the next pass
    // refills the buffer this one's draws read.
    let end_recording = builder.label("vk_end_recording");
    emit_branch_on_pass(
        builder,
        off_pass,
        CANVAS_DRAW_PASS_MORE,
        true,
        &end_recording,
    );
    emit_struct(
        builder,
        off_copy,
        COPY_SIZE,
        &[
            (COPY_ASPECT, Field::U32(IMAGE_ASPECT_COLOR)),
            (COPY_LAYER_COUNT, Field::U32("1")),
            (COPY_EXTENT_DEPTH, Field::U32("1")),
        ],
    );
    for (field, parked) in [
        (COPY_EXTENT_WIDTH, off_width),
        (COPY_EXTENT_HEIGHT, off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_copy + field,
        ));
    }
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdCopyImageToBuffer",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READ_BUFFER,
        abi::c_arg(3),
    );
    emit_int_arg(builder, platform, 4, "1");
    emit_addr_arg(builder, platform, 5, off_copy);
    emit_call_fn(builder, off_fn);

    builder.emit(abi::label(&end_recording));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkEndCommandBuffer",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_call_fn(builder, off_fn);

    // --- submit and wait ----------------------------------------------------------
    emit_struct(
        builder,
        off_submit,
        SUBMIT_INFO_SIZE,
        &[
            (0, Field::U32(ST_SUBMIT_INFO)),
            (SUBMIT_COMMAND_COUNT, Field::U32("1")),
            (SUBMIT_COMMANDS, Field::Addr(off_cmd_handle)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueSubmit",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "1"));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_submit,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_call_fn(builder, off_fn);

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueWaitIdle",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    emit_call_fn(builder, off_fn);
    // bug-765: nothing was copied out of a pass with more to come.
    emit_branch_on_pass(builder, off_pass, CANVAS_DRAW_PASS_MORE, true, &done);

    // --- BGRA -> RGBA into the surface ---------------------------------------------
    // The attachment is the layer-compatible format, so the readback is B,G,R,A while
    // the software surface — and every consumer of it — is R,G,B,A. The Metal path
    // does the same swap for the same reason.
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_MAPPED,
        abi::SCRATCH[2],
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        off_surface,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_width,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_height,
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[5], "Integer", "0"));
    builder.emit(abi::label(&swizzle_head));
    builder.emit(abi::compare_registers(abi::SCRATCH[5], abi::SCRATCH[4]));
    builder.emit(abi::branch_ge(&swizzle_done));
    builder.emit(abi::load_u8(abi::SCRATCH[6], abi::SCRATCH[2], 2)); // R
    builder.emit(abi::store_u8(abi::SCRATCH[6], abi::SCRATCH[3], 0));
    builder.emit(abi::load_u8(abi::SCRATCH[6], abi::SCRATCH[2], 1)); // G
    builder.emit(abi::store_u8(abi::SCRATCH[6], abi::SCRATCH[3], 1));
    builder.emit(abi::load_u8(abi::SCRATCH[6], abi::SCRATCH[2], 0)); // B
    builder.emit(abi::store_u8(abi::SCRATCH[6], abi::SCRATCH[3], 2));
    builder.emit(abi::load_u8(abi::SCRATCH[6], abi::SCRATCH[2], 3)); // A
    builder.emit(abi::store_u8(abi::SCRATCH[6], abi::SCRATCH[3], 3));
    builder.emit(abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 4));
    builder.emit(abi::add_immediate(abi::SCRATCH[3], abi::SCRATCH[3], 4));
    builder.emit(abi::add_immediate(abi::SCRATCH[5], abi::SCRATCH[5], 1));
    builder.emit(abi::branch(&swizzle_head));
    builder.emit(abi::label(&swizzle_done));
    builder.emit(abi::branch(&done));

    // A failure anywhere above leaves the surface exactly as `canvas::newSurface`
    // made it — the cleared frame, not garbage.
    builder.emit(abi::label(&unavailable));
    builder.emit(abi::label(&done));
    Ok(())
}
