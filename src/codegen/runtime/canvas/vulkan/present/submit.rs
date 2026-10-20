//! plan-176-V: the record-and-present of one frame (`emit_submit_and_present`). The shared
//! offsets, `PresentSlots` and the present entry points live in
//! `runtime/canvas/vulkan/present/mod.rs`.

use super::*;

/// End the recording, submit it — waiting on the acquire semaphore, signalling the
/// render one — present the image, and wait for the queue. On success `at.presented`
/// is 1 and the canvas-direct flag is set so `WM_PAINT` leaves the window alone.
#[allow(clippy::too_many_arguments)]
pub(in crate::codegen::runtime::canvas::vulkan) fn emit_submit_and_present(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_cmd_handle: usize,
    off_pass: usize,
    at: &PresentSlots,
    declined: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let presented = builder.label("vk_sc_presented");
    let last_pass = builder.label("vk_sc_last_pass");
    let finished = builder.label("vk_sc_pass_finished");
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkEndCommandBuffer",
        off_handle,
        off_fn,
        declined,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_call_fn(builder, off_fn);

    park_state(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE,
        at.sem_acquire,
    );
    park_state(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SEM_RENDER,
        at.sem_render,
    );
    park_state(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SWAPCHAIN,
        at.swapchain,
    );
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        STAGE_COLOR_ATTACHMENT_OUTPUT,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.wait_stage,
    ));
    emit_struct(
        builder,
        at.submit,
        SUBMIT_INFO_SIZE,
        &[
            (0, Field::U32(ST_SUBMIT_INFO)),
            (SUBMIT_WAIT_COUNT, Field::U32("1")),
            (SUBMIT_WAIT_SEMAPHORES, Field::Addr(at.sem_acquire)),
            (SUBMIT_WAIT_STAGES, Field::Addr(at.wait_stage)),
            (SUBMIT_COMMAND_COUNT, Field::U32("1")),
            (SUBMIT_COMMANDS, Field::Addr(off_cmd_handle)),
            (SUBMIT_SIGNAL_COUNT, Field::U32("1")),
            (SUBMIT_SIGNAL_SEMAPHORES, Field::Addr(at.sem_render)),
        ],
    );
    // bug-765: a frame's passes share one acquire and one present. Only its FIRST pass
    // waits on the acquire semaphore (a later one finds the image already its own), and
    // only its LAST signals the render semaphore the present waits on.
    {
        let waits = builder.label("vk_sc_submit_waits");
        let signals = builder.label("vk_sc_submit_signals");
        emit_branch_on_pass(builder, off_pass, CANVAS_DRAW_PASS_CONTINUES, false, &waits);
        builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.submit + SUBMIT_WAIT_COUNT,
        ));
        builder.emit(abi::label(&waits));
        emit_branch_on_pass(builder, off_pass, CANVAS_DRAW_PASS_MORE, false, &signals);
        builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.submit + SUBMIT_SIGNAL_COUNT,
        ));
        builder.emit(abi::label(&signals));
    }
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueSubmit",
        off_handle,
        off_fn,
        declined,
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
        at.submit,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(declined));

    // bug-765: a pass with more of its frame to come stops here. It waits for the
    // queue, since the next pass refills the buffer its draws read and re-records the
    // one command buffer; it leaves the image HELD for that pass; and it answers 1,
    // "this pass is in the image". Nothing is presented, so the image the window shows
    // is still the frame before: a swapchain image reaches the screen only through
    // `vkQueuePresentKHR`, which only a frame's last pass calls.
    emit_branch_on_pass(builder, off_pass, CANVAS_DRAW_PASS_MORE, false, &last_pass);
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueWaitIdle",
        off_handle,
        off_fn,
        declined,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    emit_call_fn(builder, off_fn);
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.index,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_HELD,
        abi::SCRATCH[0],
    );
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.presented,
    ));
    builder.emit(abi::branch(&finished));
    builder.emit(abi::label(&last_pass));
    // The image goes to the queue now, held by no later pass.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_HELD,
        abi::SCRATCH[0],
    );

    emit_struct(
        builder,
        at.present,
        PRESENT_INFO_SIZE,
        &[
            (0, Field::U32(ST_PRESENT_INFO_KHR)),
            (PRESENT_WAIT_COUNT, Field::U32("1")),
            (PRESENT_WAIT_SEMAPHORES, Field::Addr(at.sem_render)),
            (PRESENT_SWAPCHAIN_COUNT, Field::U32("1")),
            (PRESENT_SWAPCHAINS, Field::Addr(at.swapchain)),
            (PRESENT_IMAGE_INDICES, Field::Addr(at.index)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueuePresentKHR",
        off_handle,
        off_fn,
        declined,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        at.present,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        at.scratch,
    ));
    // Wait for the frame either way: the next one re-records the one command buffer.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkQueueWaitIdle",
        off_handle,
        off_fn,
        declined,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::c_arg(0),
    );
    emit_call_fn(builder, off_fn);
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.scratch,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&presented));
    // Suboptimal still presented; out of date did not. Rebuild before the next frame
    // either way.
    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_STALE,
        abi::SCRATCH[1],
    );
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        RESULT_SUBOPTIMAL,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_ne(declined));
    builder.emit(abi::label(&presented));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.presented,
    ));
    // PostMessageW(canvasHwnd, WM_APP_CANVAS_SHOW, 0, 0): the UI thread shows the
    // child if canvas mode is still on. Posted every frame rather than once, because
    // a readback frame or a mode change may have hidden it since; showing a visible
    // window is a no-op. Nothing to post to once the window has left canvas mode.
    let posted = builder.label("vk_sc_show_posted");
    emit_data_address(
        &symbol,
        abi::c_arg(0),
        CANVAS_HWND_SYMBOL,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(abi::c_arg(0), abi::c_arg(0), 0));
    builder.emit(abi::compare_immediate(abi::c_arg(0), "0"));
    builder.emit(abi::branch_eq(&posted));
    builder.emit(abi::move_immediate(
        abi::c_arg(1),
        "Integer",
        WM_APP_CANVAS_SHOW,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    platform.emit_external_call(
        "PostMessageW",
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::label(&posted));
    builder.emit(abi::label(&finished));
    Ok(())
}
