//! plan-176-V: `emit_vulkan_draw_scene`'s finish — draw list, present or read-back. Called in order by `emit_vulkan_draw_scene` (`runtime/canvas/vulkan/draw_scene/mod.rs`).

use super::*;

/// `emit_vulkan_draw_scene` phase 5: issue the draw list, end the pass, then present, or copy
/// out, submit, wait and swizzle the read-back.
pub(super) fn emit_finish_frame(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    s: &DrawSceneSlots,
) -> Result<(), String> {
    // plan-116-H: every block is now published, so the second pass issues every draw.
    // This replaces the trailing flush *and* the two removed above — one walk over the
    // draw list, in scene order, one instanced call per entry with that entry's group
    // offset pushed first.
    emit_draw_list_pass(
        builder,
        platform,
        platform_imports,
        s.off_state,
        s.off_handle,
        s.off_cmd_handle,
        s.off_draw_fn,
        s.off_draws,
        s.off_draw_entries,
        &s.unavailable,
    )?;

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdEndRenderPass",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    emit_call_fn(builder, s.off_fn);

    if let Some(at) = &s.swapchain {
        present::emit_submit_and_present(
            builder,
            platform,
            platform_imports,
            s.off_handle,
            s.off_fn,
            s.off_state,
            s.off_cmd_handle,
            s.off_pass,
            at,
            &s.unavailable,
        )?;
        builder.emit(abi::branch(&s.done));
        builder.emit(abi::label(&s.unavailable));
        present::emit_present_abandon(
            builder,
            platform,
            platform_imports,
            s.off_handle,
            s.off_fn,
            s.off_state,
            at,
        )?;
        builder.emit(abi::label(&s.done));
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
        s.off_pass,
        CANVAS_DRAW_PASS_MORE,
        true,
        &end_recording,
    );
    emit_struct(
        builder,
        s.off_copy,
        COPY_SIZE,
        &[
            (COPY_ASPECT, Field::U32(IMAGE_ASPECT_COLOR)),
            (COPY_LAYER_COUNT, Field::U32("1")),
            (COPY_EXTENT_DEPTH, Field::U32("1")),
        ],
    );
    for (field, parked) in [
        (COPY_EXTENT_WIDTH, s.off_width),
        (COPY_EXTENT_HEIGHT, s.off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            s.off_copy + field,
        ));
    }
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdCopyImageToBuffer",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    emit_state_load(
        builder,
        s.off_state,
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
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_READ_BUFFER,
        abi::c_arg(3),
    );
    emit_int_arg(builder, platform, 4, "1");
    emit_addr_arg(builder, platform, 5, s.off_copy);
    emit_call_fn(builder, s.off_fn);

    builder.emit(abi::label(&end_recording));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkEndCommandBuffer",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        s.off_cmd_handle,
    ));
    emit_call_fn(builder, s.off_fn);

    // --- submit and wait ----------------------------------------------------------
    emit_struct(
        builder,
        s.off_submit,
        SUBMIT_INFO_SIZE,
        &[
            (0, Field::U32(ST_SUBMIT_INFO)),
            (SUBMIT_COMMAND_COUNT, Field::U32("1")),
            (SUBMIT_COMMANDS, Field::Addr(s.off_cmd_handle)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueSubmit",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "1"));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        s.off_submit,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_call_fn(builder, s.off_fn);

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueWaitIdle",
        s.off_handle,
        s.off_fn,
        &s.unavailable,
    )?;
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    emit_call_fn(builder, s.off_fn);
    // bug-765: nothing was copied out of a pass with more to come.
    emit_branch_on_pass(builder, s.off_pass, CANVAS_DRAW_PASS_MORE, true, &s.done);

    // --- BGRA -> RGBA into the surface ---------------------------------------------
    // The attachment is the layer-compatible format, so the readback is B,G,R,A while
    // the software surface — and every consumer of it — is R,G,B,A. The Metal path
    // does the same swap for the same reason.
    emit_state_load(
        builder,
        s.off_state,
        GRAPHICS_OFFSET_VULKAN_MAPPED,
        abi::SCRATCH[2],
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        s.off_surface,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        s.off_width,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        s.off_height,
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[5], "Integer", "0"));
    builder.emit(abi::label(&s.swizzle_head));
    builder.emit(abi::compare_registers(abi::SCRATCH[5], abi::SCRATCH[4]));
    builder.emit(abi::branch_ge(&s.swizzle_done));
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
    builder.emit(abi::branch(&s.swizzle_head));
    builder.emit(abi::label(&s.swizzle_done));
    builder.emit(abi::branch(&s.done));

    // A failure anywhere above leaves the surface exactly as `canvas::newSurface`
    // made it — the cleared frame, not garbage.
    builder.emit(abi::label(&s.unavailable));
    builder.emit(abi::label(&s.done));
    Ok(())
}
