//! plan-176-V: the graphics thread's render target (`emit_vulkan_target`). The shared
//! constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// Ensure the offscreen image, its framebuffer and the readback buffer exist at
/// `width` x `height`, rebuilding them if the surface has resized.
///
/// The Vulkan counterpart of the Metal path's texture check, and the same shape: the
/// dimensions live beside the handles so the common case is two word compares rather
/// than a query, and a resize tears the old set down before building the new one.
/// Leaking here would be a whole surface's worth of device memory per resize step —
/// megabytes per frame of a window drag.
///
/// The command pool and its single command buffer are built here too, but only once:
/// neither depends on the surface size, and the pool is created with
/// `RESET_COMMAND_BUFFER` so each frame re-records the same buffer instead of
/// allocating one.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_vulkan_target(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    off_width: usize,
    off_height: usize,
    unavailable: &str,
) -> Result<(), String> {
    let build = builder.label("vk_target_build");
    let ready = builder.label("vk_target_ready");
    let no_teardown = builder.label("vk_target_no_teardown");
    let have_pool = builder.label("vk_target_have_pool");

    let off_image_info = builder.allocate_stack_object("vk_image_info", IMAGE_INFO_SIZE);
    let off_reqs = builder.allocate_stack_object("vk_reqs", MEMORY_REQS_SIZE);
    let off_alloc_info = builder.allocate_stack_object("vk_alloc_info", ALLOC_INFO_SIZE);
    let off_properties = builder.allocate_stack_object("vk_mem_properties", MEMORY_PROPERTIES_SIZE);
    let off_type_index = builder.allocate_stack_object("vk_type_index", 8);
    let off_type_bits = builder.allocate_stack_object("vk_type_bits", 8);
    let off_view_info = builder.allocate_stack_object("vk_view_info", VIEW_INFO_SIZE);
    let off_view_handle = builder.allocate_stack_object("vk_view_handle", 8);
    let off_fb_info = builder.allocate_stack_object("vk_fb_info", FRAMEBUFFER_INFO_SIZE);
    let off_buffer_info = builder.allocate_stack_object("vk_buffer_info", BUFFER_INFO_SIZE);
    let off_pool_info = builder.allocate_stack_object("vk_pool_info", POOL_INFO_SIZE);
    let off_cmd_alloc = builder.allocate_stack_object("vk_cmd_alloc", CMD_ALLOC_INFO_SIZE);
    let off_bytes = builder.allocate_stack_object("vk_bytes", 8);

    // Already the right size?
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&build));
    for (slot, parked) in [
        (GRAPHICS_OFFSET_VULKAN_TEX_WIDTH, off_width),
        (GRAPHICS_OFFSET_VULKAN_TEX_HEIGHT, off_height),
    ] {
        emit_state_load(builder, off_state, slot, abi::SCRATCH[0]);
        builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        builder.emit(abi::branch_ne(&build));
    }
    builder.emit(abi::branch(&ready));

    builder.emit(abi::label(&build));
    // Tear the old set down first. Everything the GPU could still be using is behind
    // a `vkDeviceWaitIdle`, which is the blunt instrument and the right one here: a
    // resize is rare and this is not the frame path.
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&no_teardown));
    emit_device_call_1(
        builder,
        platform,
        platform_imports,
        "vkDeviceWaitIdle",
        off_handle,
        off_fn,
        off_state,
        unavailable,
    )?;
    for (name, slot) in [
        ("vkDestroyFramebuffer", GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER),
        ("vkDestroyImageView", GRAPHICS_OFFSET_VULKAN_IMAGE_VIEW),
        ("vkDestroyImage", GRAPHICS_OFFSET_VULKAN_IMAGE),
        ("vkDestroyBuffer", GRAPHICS_OFFSET_VULKAN_READ_BUFFER),
        ("vkFreeMemory", GRAPHICS_OFFSET_VULKAN_IMAGE_MEMORY),
        ("vkFreeMemory", GRAPHICS_OFFSET_VULKAN_READ_MEMORY),
    ] {
        emit_dlsym(
            builder,
            platform,
            platform_imports,
            name,
            off_handle,
            off_fn,
            unavailable,
        )?;
        emit_state_load(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_DEVICE,
            abi::c_arg(0),
        );
        emit_state_load(builder, off_state, slot, abi::c_arg(1));
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
        emit_call_fn(builder, off_fn);
        builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        emit_state_store(builder, off_state, slot, abi::SCRATCH[0]);
    }

    builder.emit(abi::label(&no_teardown));

    // bytes = width * height * 4
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
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "4"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_bytes,
    ));

    // --- the colour image ---------------------------------------------------------
    emit_struct(
        builder,
        off_image_info,
        IMAGE_INFO_SIZE,
        &[
            (0, Field::U32(ST_IMAGE_CREATE_INFO)),
            (IMAGE_INFO_TYPE, Field::U32(IMAGE_TYPE_2D)),
            (IMAGE_INFO_FORMAT, Field::U32(FORMAT_B8G8R8A8_SRGB)),
            (IMAGE_INFO_DEPTH, Field::U32("1")),
            (IMAGE_INFO_MIP_LEVELS, Field::U32("1")),
            (IMAGE_INFO_ARRAY_LAYERS, Field::U32("1")),
            (IMAGE_INFO_SAMPLES, Field::U32(SAMPLE_COUNT_1)),
            (IMAGE_INFO_TILING, Field::U32(IMAGE_TILING_OPTIMAL)),
            (IMAGE_INFO_USAGE, Field::U32(IMAGE_USAGE_COLOR_AND_SRC)),
            (
                IMAGE_INFO_INITIAL_LAYOUT,
                Field::U32(IMAGE_LAYOUT_UNDEFINED),
            ),
        ],
    );
    for (field, parked) in [
        (IMAGE_INFO_WIDTH, off_width),
        (IMAGE_INFO_HEIGHT, off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_image_info + field,
        ));
    }
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateImage",
        off_handle,
        off_fn,
        off_state,
        off_image_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        unavailable,
    )?;

    // Its memory.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetImageMemoryRequirements",
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        abi::c_arg(1),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_reqs,
    ));
    emit_call_fn(builder, off_fn);
    emit_allocate_and_bind(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_reqs,
        off_alloc_info,
        off_properties,
        off_type_index,
        off_type_bits,
        off_out,
        &[MEMORY_DEVICE_LOCAL],
        GRAPHICS_OFFSET_VULKAN_IMAGE_MEMORY,
        "vkBindImageMemory",
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        unavailable,
    )?;

    // --- the view and the framebuffer ---------------------------------------------
    emit_struct(
        builder,
        off_view_info,
        VIEW_INFO_SIZE,
        &[
            (0, Field::U32(ST_IMAGE_VIEW_CREATE_INFO)),
            (VIEW_INFO_TYPE, Field::U32(IMAGE_VIEW_TYPE_2D)),
            (VIEW_INFO_FORMAT, Field::U32(FORMAT_B8G8R8A8_SRGB)),
            (VIEW_INFO_ASPECT, Field::U32(IMAGE_ASPECT_COLOR)),
            (VIEW_INFO_LEVEL_COUNT, Field::U32("1")),
            (VIEW_INFO_LAYER_COUNT, Field::U32("1")),
        ],
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_IMAGE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_view_info + VIEW_INFO_IMAGE,
    ));
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateImageView",
        off_handle,
        off_fn,
        off_state,
        off_view_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_IMAGE_VIEW,
        unavailable,
    )?;

    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_IMAGE_VIEW,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_view_handle,
    ));
    emit_struct(
        builder,
        off_fb_info,
        FRAMEBUFFER_INFO_SIZE,
        &[
            (0, Field::U32(ST_FRAMEBUFFER_CREATE_INFO)),
            (FRAMEBUFFER_ATTACHMENT_COUNT, Field::U32("1")),
            (FRAMEBUFFER_ATTACHMENTS, Field::Addr(off_view_handle)),
            (FRAMEBUFFER_LAYERS, Field::U32("1")),
        ],
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_RENDER_PASS,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_fb_info + FRAMEBUFFER_RENDER_PASS,
    ));
    for (field, parked) in [
        (FRAMEBUFFER_WIDTH, off_width),
        (FRAMEBUFFER_HEIGHT, off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_fb_info + field,
        ));
    }
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateFramebuffer",
        off_handle,
        off_fn,
        off_state,
        off_fb_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER,
        unavailable,
    )?;

    // --- the readback buffer ------------------------------------------------------
    emit_struct(
        builder,
        off_buffer_info,
        BUFFER_INFO_SIZE,
        &[
            (0, Field::U32(ST_BUFFER_CREATE_INFO)),
            (BUFFER_INFO_USAGE, Field::U32(BUFFER_USAGE_TRANSFER_DST)),
        ],
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_bytes,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_buffer_info + BUFFER_INFO_BYTES,
    ));
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateBuffer",
        off_handle,
        off_fn,
        off_state,
        off_buffer_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_READ_BUFFER,
        unavailable,
    )?;
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetBufferMemoryRequirements",
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READ_BUFFER,
        abi::c_arg(1),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_reqs,
    ));
    emit_call_fn(builder, off_fn);
    emit_allocate_and_bind(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_reqs,
        off_alloc_info,
        off_properties,
        off_type_index,
        off_type_bits,
        off_out,
        // bug-688 V5: CPU-read every frame, so cached if the device offers it.
        &READBACK_MEMORY_CANDIDATES,
        GRAPHICS_OFFSET_VULKAN_READ_MEMORY,
        "vkBindBufferMemory",
        GRAPHICS_OFFSET_VULKAN_READ_BUFFER,
        unavailable,
    )?;

    // Map it once and keep the pointer: Vulkan allows a HOST_VISIBLE allocation to
    // stay mapped for its lifetime, so this saves a map/unmap pair every frame.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkMapMemory",
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READ_MEMORY,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0")); // offset
    builder.emit(abi::load_u64(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_bytes,
    ));
    emit_int_arg(builder, platform, 4, "0"); // flags
    emit_addr_arg(builder, platform, 5, off_out);
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(unavailable));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_MAPPED,
        abi::SCRATCH[0],
    );

    // Remember the size this set was built for.
    for (slot, parked) in [
        (GRAPHICS_OFFSET_VULKAN_TEX_WIDTH, off_width),
        (GRAPHICS_OFFSET_VULKAN_TEX_HEIGHT, off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        emit_state_store(builder, off_state, slot, abi::SCRATCH[0]);
    }

    // --- the command pool and its one buffer, built once --------------------------
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_COMMAND_POOL,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(&have_pool));
    emit_struct(
        builder,
        off_pool_info,
        POOL_INFO_SIZE,
        &[
            (0, Field::U32(ST_COMMAND_POOL_CREATE_INFO)),
            (POOL_INFO_FLAGS, Field::U32(POOL_RESET_COMMAND_BUFFER)),
        ],
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_pool_info + POOL_INFO_FAMILY,
    ));
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateCommandPool",
        off_handle,
        off_fn,
        off_state,
        off_pool_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_COMMAND_POOL,
        unavailable,
    )?;
    emit_struct(
        builder,
        off_cmd_alloc,
        CMD_ALLOC_INFO_SIZE,
        &[
            (0, Field::U32(ST_COMMAND_BUFFER_ALLOCATE_INFO)),
            (CMD_ALLOC_LEVEL, Field::U32(COMMAND_BUFFER_LEVEL_PRIMARY)),
            (CMD_ALLOC_COUNT, Field::U32("1")),
        ],
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_COMMAND_POOL,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_cmd_alloc + CMD_ALLOC_POOL,
    ));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkAllocateCommandBuffers",
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_cmd_alloc,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(unavailable));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER,
        abi::SCRATCH[0],
    );

    builder.emit(abi::label(&have_pool));
    builder.emit(abi::label(&ready));
    Ok(())
}
