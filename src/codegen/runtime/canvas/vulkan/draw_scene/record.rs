//! plan-176-V: `emit_vulkan_draw_scene`'s record and item walk. Called in order by `emit_vulkan_draw_scene` (`runtime/canvas/vulkan/draw_scene/mod.rs`).

use super::*;

/// `emit_vulkan_draw_scene` phase 3: begin the command buffer and the render pass, bind the
/// pipeline and descriptor sets, set the viewport and scissor, and resolve the per-item draws.
pub(super) fn emit_record_begin(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    s: &DrawSceneSlots,
    off_render_pass: usize,
) -> Result<(), String> {
    // --- record ------------------------------------------------------------------
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    emit_struct(
        builder,
        s.off_begin,
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
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        s.off_begin,
    ));
    emit_call_fn(builder, s.off_fn);

    // Opaque black, matching `canvas::newSurface`, so both backends start from the
    // same pixels without either naming the colour twice.
    emit_struct(
        builder,
        s.off_clear,
        CLEAR_VALUE_SIZE,
        &[(CLEAR_ALPHA, Field::U32(FLOAT_ONE_BITS_F32))],
    );
    emit_struct(
        builder,
        s.off_pass_begin,
        PASS_BEGIN_INFO_SIZE,
        &[
            (0, Field::U32(ST_RENDER_PASS_BEGIN_INFO)),
            (PASS_BEGIN_CLEAR_COUNT, Field::U32("1")),
            (PASS_BEGIN_CLEARS, Field::Addr(s.off_clear)),
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
        s.off_pass_begin + PASS_BEGIN_RENDER_PASS,
    ));
    match &s.swapchain {
        Some(at) => builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.framebuffer,
        )),
        None => emit_state_load(
            builder,
            s.off_state,
            GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER,
            abi::SCRATCH[0],
        ),
    }
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_pass_begin + PASS_BEGIN_FRAMEBUFFER,
    ));
    for (field, parked) in [
        (PASS_BEGIN_AREA_WIDTH, s.off_width),
        (PASS_BEGIN_AREA_HEIGHT, s.off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_pass_begin + field,
        ));
    }
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBeginRenderPass",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        s.off_pass_begin,
    ));
    builder.emit(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        SUBPASS_CONTENTS_INLINE,
    ));
    emit_call_fn(builder, s.off_fn);

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBindPipeline",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        PIPELINE_BIND_POINT_GRAPHICS,
    ));
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_PIPELINE,
        abi::c_arg(2),
    );
    emit_call_fn(builder, s.off_fn);

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
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        PIPELINE_BIND_POINT_GRAPHICS,
    ));
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
        abi::c_arg(2),
    );
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // firstSet
    emit_int_arg(builder, platform, 4, "1"); // setCount
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_DESC_SET,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_desc_set_handle,
    ));
    emit_addr_arg(builder, platform, 5, s.off_desc_set_handle);
    emit_int_arg_zero(builder, platform, 6); // dynamicOffsetCount
    emit_int_arg_zero(builder, platform, 7); // pDynamicOffsets
    emit_call_fn(builder, s.off_fn);

    // Viewport and scissor are dynamic state, so they are set per frame — which is
    // exactly what lets a resize reuse the pipeline.
    emit_struct(
        builder,
        s.off_viewport,
        VIEWPORT_SIZE,
        &[(VIEWPORT_MAX_DEPTH, Field::U32(FLOAT_ONE_BITS_F32))],
    );
    for (field, parked) in [
        (VIEWPORT_WIDTH, s.off_width),
        (VIEWPORT_HEIGHT, s.off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        emit_store_f32_from_integer(builder, abi::SCRATCH[0], s.off_viewport + field);
    }
    emit_struct(builder, s.off_scissor, RECT_SIZE, &[]);
    for (field, parked) in [(RECT_WIDTH, s.off_width), (RECT_HEIGHT, s.off_height)] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_scissor + field,
        ));
    }
    for (name, slot) in [
        ("vkCmdSetViewport", s.off_viewport),
        ("vkCmdSetScissor", s.off_scissor),
    ] {
        emit_dlsym(
            builder,
            platform,
            platform_imports,
            name,
            s.off_handle,
            s.off_fn,
            &s.unavailable,
        )?;
        builder.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            s.off_cmd_handle,
        ));
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
        builder.emit(abi::add_immediate(
            abi::c_arg(3),
            abi::stack_pointer(),
            slot,
        ));
        emit_call_fn(builder, s.off_fn);
    }

    // `vkCmdDraw`, resolved once. Every draw in the frame goes through this one slot
    // now — the run flushes and the per-glyph draws alike — so the shape path no longer
    // re-resolves anything per item.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdDraw",
        s.off_handle,
        s.off_draw_fn,
        &s.unavailable,
    )?;
    // `vkCmdBindPipeline`, resolved once for the same reason: it is now called per RUN
    // rather than once per frame, and a scene that alternates blend modes would put a
    // `dlsym` between every pair of items.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdBindPipeline",
        s.off_handle,
        s.off_pipe_fn,
        &s.unavailable,
    )?;
    Ok(())
}

/// `emit_vulkan_draw_scene` phase 4: walk the items, one quad (or glyph run) per item,
/// rebinding the pipeline when the blend mode changes.
pub(super) fn emit_item_walk(builder: &mut CodeBuilder, s: &DrawSceneSlots) {
    // --- one quad per item --------------------------------------------------------
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_index,
    ));
    // The frame's running edge cursor: each polygon appends its edges here and
    // records where they started in its own item block.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_edge_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_glyph_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_grad_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_dash_cursor,
    ));
    // bug-688: an empty picture table. Nothing in the table itself is cleared -- a slot
    // is live only below this count (`emit_picture_lookup`).
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_pic_count,
    ));
    // bug-688: and an empty band region.
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.bands.cursor,
    ));
    // The item-buffer cursor and the current run's base, both starting at quad zero.
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_item_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_run_start,
    ));
    // `Normal` is what the once-per-frame bind above left bound, so the frame starts
    // knowing that. An all-`Normal` scene therefore issues exactly the one
    // `vkCmdBindPipeline` it always did.
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_bound_mode,
    ));
    builder.emit(abi::label(&s.item_head));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_index,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        s.off_count,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_ge(&s.item_done));

    // header = geometry + offsets[i] * 8
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        s.off_offsets,
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
        s.off_geometry,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_header,
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
            s.off_item_mode,
        ));
        builder.emit(abi::load_u64(
            abi::SCRATCH[2],
            abi::stack_pointer(),
            s.off_bound_mode,
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
            s.off_item_mode,
        ));
        builder.emit(abi::shift_left_immediate(
            abi::SCRATCH[0],
            abi::SCRATCH[0],
            3,
        ));
        builder.emit(abi::load_u64(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            s.off_state,
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
            s.off_run_count,
        ));
        builder.emit(abi::load_u64(
            abi::c_arg(0),
            abi::stack_pointer(),
            s.off_cmd_handle,
        ));
        builder.emit(abi::move_immediate(
            abi::c_arg(1),
            "Integer",
            PIPELINE_BIND_POINT_GRAPHICS,
        ));
        builder.emit(abi::load_u64(
            abi::c_arg(2),
            abi::stack_pointer(),
            s.off_run_count,
        ));
        emit_call_fn(builder, s.off_pipe_fn);

        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_item_mode,
        ));
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_bound_mode,
        ));
        builder.emit(abi::label(&same_mode));
        // The header address was in SCRATCH[0] on entry and the block above used it;
        // restore it before the kind fork reads it.
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_header,
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
    builder.emit(abi::branch_eq(&s.text_item));

    emit_item_block(builder, s.off_item, s.off_width, s.off_height);
    emit_edge_upload(
        builder,
        s.off_state,
        s.off_item,
        s.off_header,
        s.off_edge_cursor,
        &s.bands,
    );
    // plan-163-B: after the item's own edges, so its join records follow them; then the
    // dash words, after the join upload has zeroed their block words.
    emit_join_upload(
        builder,
        s.off_state,
        s.off_item,
        s.off_header,
        s.off_edge_cursor,
    );
    emit_dash_upload(
        builder,
        s.off_state,
        s.off_item,
        s.off_header,
        s.off_dash_cursor,
    );
    // plan-163-C: the clip's words and a path clip's edges, after the join records.
    emit_clip_upload(
        builder,
        s.off_state,
        s.off_item,
        s.off_header,
        s.off_edge_cursor,
        &s.bands,
    );
    emit_gradient_upload(
        builder,
        s.off_state,
        s.off_item,
        s.off_header,
        s.off_grad_cursor,
    );
    // bug-484: after the edge upload (which zeroes the slice fields for non-polygons)
    // and before the split, so both records of a split picture name its texels.
    emit_picture_upload(
        builder,
        PictureSlots {
            state: s.off_state,
            item: s.off_item,
            header: s.off_header,
            glyph_cursor: s.off_glyph_cursor,
            pic_count: s.off_pic_count,
            pic_slot: s.off_pic_slot,
        },
    );
    // Published, not drawn. The draw happens at the end of the run this item joins —
    // which is what makes consecutive shapes one instanced `vkCmdDraw` instead of N.
    emit_split_or_publish(
        builder,
        s.off_state,
        s.off_item,
        s.off_item_cursor,
        s.off_item_mode,
        s.off_saved_stroke,
        &s.item_next,
    );
    builder.emit(abi::branch(&s.item_next));

    // plan-116-H: no flush here either. Ordering used to be maintained by issuing the
    // accumulated run before the glyphs; it is now maintained by the draw list being in
    // scene order and the second pass walking it in that order. A `Text` item is its own
    // entry (H8), so nothing can merge across it.
    builder.emit(abi::label(&s.text_item));
    emit_glyph_publish(
        builder,
        GlyphPublishSlots {
            state: s.off_state,
            item: s.off_item,
            header: s.off_header,
            width: s.off_width,
            height: s.off_height,
            item_cursor: s.off_item_cursor,
            instance: s.off_glyph_instance,
            glyph_meta: s.off_glyph_meta,
            glyph_cov: s.off_glyph_cov,
            glyph_cursor: s.off_glyph_cursor,
            glyph_index: s.off_glyph_index,
            glyph_count: s.off_glyph_count,
            glyph_w: s.off_glyph_w,
            glyph_h: s.off_glyph_h,
            glyph_x: s.off_glyph_x,
            glyph_y: s.off_glyph_y,
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
        s.off_item_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_run_start,
    ));

    builder.emit(abi::label(&s.item_next));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_index,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_index,
    ));
    builder.emit(abi::branch(&s.item_head));
    builder.emit(abi::label(&s.item_done));
}
