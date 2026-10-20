//! plan-176-V: the swapchain (re)creation (`emit_ensure_swapchain`). The shared offsets,
//! `PresentSlots` and the present entry points live in `runtime/canvas/vulkan/present/mod.rs`.

use super::*;

/// Make sure a swapchain for the canvas window exists at `width` x `height`, and
/// acquire its next image: on return the image's index and framebuffer are in
/// `at.index` / `at.framebuffer` and the acquire semaphore will signal. Branches to
/// `declined` whenever this frame cannot be presented directly — the caller renders it
/// down the readback path instead.
#[allow(clippy::too_many_arguments)]
pub(in crate::codegen::runtime::canvas::vulkan) fn emit_ensure_swapchain(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    off_width: usize,
    off_height: usize,
    at: &PresentSlots,
    declined: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let failed = builder.label("vk_sc_failed");
    let have_swapchain = builder.label("vk_sc_have_swapchain");
    let rebuild = builder.label("vk_sc_rebuild");
    let extent_ok = builder.label("vk_sc_extent_ok");
    let alpha_ok = builder.label("vk_sc_alpha_ok");
    let format_head = builder.label("vk_sc_format_head");
    let format_found = builder.label("vk_sc_format_found");
    let acquired = builder.label("vk_sc_acquired");
    let stale_acquire = builder.label("vk_sc_stale_acquire");

    // A device or surface that can never present, or no window-system extensions.
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_FAILED,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(declined));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_WSI,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "1"));
    builder.emit(abi::branch_ne(declined));

    // No canvas window on screen (headless, or canvas mode left): readback.
    emit_data_address(
        &symbol,
        abi::SCRATCH[0],
        CANVAS_HWND_SYMBOL,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(declined));
    // The surface is the child window, which the canvas reconcile creates.
    emit_data_address(
        &symbol,
        abi::SCRATCH[0],
        CANVAS_CHILD_SYMBOL,
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(abi::SCRATCH[0], abi::SCRATCH[0], 0));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.hwnd,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(declined));

    // Present with what exists when all of it is current: a swapchain on a surface for
    // this window, not marked stale, at the canvas size.
    for (slot, test) in [
        (GRAPHICS_OFFSET_VULKAN_SURFACE, "exists"),
        (GRAPHICS_OFFSET_VULKAN_SWAPCHAIN, "exists"),
        (GRAPHICS_OFFSET_VULKAN_SC_STALE, "clear"),
    ] {
        emit_state_load(builder, off_state, slot, abi::SCRATCH[0]);
        builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
        builder.emit(if test == "exists" {
            abi::branch_eq(&rebuild)
        } else {
            abi::branch_ne(&rebuild)
        });
    }
    for (state, parked) in [
        (GRAPHICS_OFFSET_VULKAN_SURFACE_HWND, at.hwnd),
        (GRAPHICS_OFFSET_VULKAN_SC_W, off_width),
        (GRAPHICS_OFFSET_VULKAN_SC_H, off_height),
    ] {
        emit_state_load(builder, off_state, state, abi::SCRATCH[0]);
        builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        builder.emit(abi::branch_ne(&rebuild));
    }
    builder.emit(abi::branch(&have_swapchain));

    // --- otherwise everything is rebuilt: surface, swapchain, views, framebuffers ----
    //
    // One build path, from nothing, rather than a swapchain rebuilt on a kept surface
    // with `oldSwapchain` handing the images over: no retired swapchain to track, no
    // partial state for a failed rebuild to leave behind, and the same code serves the
    // first frame, a resize, a canvas re-entry and an out-of-date image. None of those
    // is per frame, so the extra surface creation costs nothing measurable.
    builder.emit(abi::label(&rebuild));
    emit_teardown(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        at,
        declined,
    )?;
    emit_struct(
        builder,
        at.info,
        WIN32_SURFACE_INFO_SIZE,
        &[(0, Field::U32(ST_WIN32_SURFACE_CREATE_INFO_KHR))],
    );
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
    platform.emit_external_call(
        "GetModuleHandleW",
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        at.info + WIN32_SURFACE_HINSTANCE,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.hwnd,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.info + WIN32_SURFACE_HWND,
    ));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreateWin32SurfaceKHR",
        off_handle,
        off_fn,
        &failed,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_INSTANCE,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        at.info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&failed));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.hwnd,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SURFACE_HWND,
        abi::SCRATCH[0],
    );

    // The queue family must be able to present to it.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetPhysicalDeviceSurfaceSupportKHR",
        off_handle,
        off_fn,
        &failed,
    )?;
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.scratch,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY,
        abi::c_arg(1),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        abi::c_arg(2),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        at.scratch,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.scratch,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&failed));

    // The surface must take the attachment format the pipelines were built for.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetPhysicalDeviceSurfaceFormatsKHR",
        off_handle,
        off_fn,
        &failed,
    )?;
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &MAX_SURFACE_FORMATS.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.count,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        abi::c_arg(1),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        at.count,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        at.formats,
    ));
    emit_call_fn(builder, off_fn);
    // VK_INCOMPLETE (5) still filled the array; anything negative is a failure.
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.count,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[2], "Integer", "0"));
    builder.emit(abi::label(&format_head));
    builder.emit(abi::compare_registers(abi::SCRATCH[2], abi::SCRATCH[1]));
    builder.emit(abi::branch_ge(&failed));
    // The array base goes through `add_immediate`: a register-form add reads
    // register 31 as the zero register, not the stack pointer.
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[3],
        abi::SCRATCH[2],
        3,
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[4],
        abi::stack_pointer(),
        at.formats,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[3],
        abi::SCRATCH[4],
        abi::SCRATCH[3],
    ));
    builder.emit(abi::load_u32(abi::SCRATCH[4], abi::SCRATCH[3], 0));
    builder.emit(abi::add_immediate(abi::SCRATCH[2], abi::SCRATCH[2], 1));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[4],
        FORMAT_B8G8R8A8_SRGB,
    ));
    builder.emit(abi::branch_ne(&format_head));
    builder.emit(abi::label(&format_found));

    emit_present_render_pass(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_out,
        at,
        &failed,
    )?;

    // --- the swapchain, its images, a view and a framebuffer for each ---------------
    // The frame's two semaphores, made on first use and again after a frame that
    // acquired an image and then could not submit it dropped them (a signalled
    // semaphore cannot be waited for twice, and there is no way to unsignal one).
    let have_semaphores = builder.label("vk_sc_have_semaphores");
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(&have_semaphores));
    for slot in [
        GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE,
        GRAPHICS_OFFSET_VULKAN_SEM_RENDER,
    ] {
        emit_struct(
            builder,
            at.info,
            SEMAPHORE_INFO_SIZE,
            &[(0, Field::U32(ST_SEMAPHORE_CREATE_INFO))],
        );
        emit_create_call(
            builder,
            platform,
            platform_imports,
            "vkCreateSemaphore",
            off_handle,
            off_fn,
            off_state,
            at.info,
            off_out,
            slot,
            declined,
        )?;
    }
    builder.emit(abi::label(&have_semaphores));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
        off_handle,
        off_fn,
        declined,
    )?;
    emit_zero_range(builder, at.caps, CAPS_SIZE);
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        abi::c_arg(1),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        at.caps,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(declined));
    // The window must be exactly the canvas size (a minimised window reports 0, and a
    // DPI-scaled client is a different size): otherwise this frame takes readback.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.caps + CAPS_CURRENT_WIDTH,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        EXTENT_UNDEFINED,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_eq(&extent_ok));
    for (field, parked) in [
        (CAPS_CURRENT_WIDTH, off_width),
        (CAPS_CURRENT_HEIGHT, off_height),
    ] {
        builder.emit(abi::load_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.caps + field,
        ));
        builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
        builder.emit(abi::branch_ne(declined));
    }
    builder.emit(abi::label(&extent_ok));

    // Image count: min + 1, within the surface's max (0 = none) and ours.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.caps + CAPS_MIN_IMAGES,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.caps + CAPS_MAX_IMAGES,
    ));
    let under_max = builder.label("vk_sc_under_max");
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], "0"));
    builder.emit(abi::branch_eq(&under_max));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_le(&under_max));
    builder.emit(abi::move_register(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::label(&under_max));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        &VULKAN_MAX_SWAPCHAIN_IMAGES.to_string(),
    ));
    let under_ours = builder.label("vk_sc_under_ours");
    builder.emit(abi::branch_le(&under_ours));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &VULKAN_MAX_SWAPCHAIN_IMAGES.to_string(),
    ));
    builder.emit(abi::label(&under_ours));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.count,
    ));
    // A surface whose MINIMUM exceeds what we track can never present here.
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.caps + CAPS_MIN_IMAGES,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[0]));
    builder.emit(abi::branch_gt(&failed));

    emit_struct(
        builder,
        at.info,
        SWAPCHAIN_INFO_SIZE,
        &[
            (0, Field::U32(ST_SWAPCHAIN_CREATE_INFO_KHR)),
            (SWAPCHAIN_FORMAT, Field::U32(FORMAT_B8G8R8A8_SRGB)),
            (
                SWAPCHAIN_COLOR_SPACE,
                Field::U32(COLOR_SPACE_SRGB_NONLINEAR),
            ),
            (SWAPCHAIN_LAYERS, Field::U32("1")),
            (SWAPCHAIN_USAGE, Field::U32(USAGE_COLOR_ATTACHMENT)),
            (SWAPCHAIN_PRESENT_MODE, Field::U32(PRESENT_MODE_FIFO)),
            (SWAPCHAIN_CLIPPED, Field::U32("1")),
        ],
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.count,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.info + SWAPCHAIN_MIN_IMAGES,
    ));
    for (field, parked) in [(SWAPCHAIN_WIDTH, off_width), (SWAPCHAIN_HEIGHT, off_height)] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.info + field,
        ));
    }
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.caps + CAPS_CURRENT_TRANSFORM,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.info + SWAPCHAIN_PRE_TRANSFORM,
    ));
    // Opaque if the surface offers it, else the lowest mode it does offer.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.caps + CAPS_COMPOSITE_ALPHA,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "1"));
    builder.emit(abi::and_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[2], "0"));
    builder.emit(abi::branch_ne(&alpha_ok));
    builder.emit(abi::move_immediate(abi::SCRATCH[2], "Integer", "0"));
    builder.emit(abi::subtract_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[2],
        abi::SCRATCH[0],
    ));
    builder.emit(abi::and_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::label(&alpha_ok));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.info + SWAPCHAIN_COMPOSITE_ALPHA,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.info + SWAPCHAIN_SURFACE,
    ));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreateSwapchainKHR",
        off_handle,
        off_fn,
        declined,
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
        at.info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(declined));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SWAPCHAIN,
        abi::SCRATCH[0],
    );

    // Its images, and a view and framebuffer for each.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetSwapchainImagesKHR",
        off_handle,
        off_fn,
        declined,
    )?;
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &VULKAN_MAX_SWAPCHAIN_IMAGES.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.count,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SWAPCHAIN,
        abi::c_arg(1),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        at.count,
    ));
    builder.emit(abi::load_u64(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_state,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::c_arg(3),
        GRAPHICS_OFFSET_VULKAN_SC_IMAGES,
    ));
    emit_call_fn(builder, off_fn);
    // VK_INCOMPLETE (5) is fine: we asked for at most our cap. Negative is not.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.count,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.count,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_COUNT,
        abi::SCRATCH[0],
    );
    for i in 0..VULKAN_MAX_SWAPCHAIN_IMAGES {
        let skip = builder.label("vk_sc_image_skip");
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.count,
        ));
        builder.emit(abi::compare_immediate(abi::SCRATCH[0], &i.to_string()));
        builder.emit(abi::branch_le(&skip));
        emit_struct(
            builder,
            at.info,
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
            GRAPHICS_OFFSET_VULKAN_SC_IMAGES + i * 8,
            abi::SCRATCH[0],
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.info + VIEW_INFO_IMAGE,
        ));
        emit_create_call(
            builder,
            platform,
            platform_imports,
            "vkCreateImageView",
            off_handle,
            off_fn,
            off_state,
            at.info,
            off_out,
            GRAPHICS_OFFSET_VULKAN_SC_VIEWS + i * 8,
            declined,
        )?;
        park_state(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_SC_VIEWS + i * 8,
            at.view_handle,
        );
        emit_struct(
            builder,
            at.info,
            FRAMEBUFFER_INFO_SIZE,
            &[
                (0, Field::U32(ST_FRAMEBUFFER_CREATE_INFO)),
                (FRAMEBUFFER_ATTACHMENT_COUNT, Field::U32("1")),
                (FRAMEBUFFER_ATTACHMENTS, Field::Addr(at.view_handle)),
                (FRAMEBUFFER_LAYERS, Field::U32("1")),
            ],
        );
        emit_state_load(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_PRESENT_PASS,
            abi::SCRATCH[0],
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.info + FRAMEBUFFER_RENDER_PASS,
        ));
        for (field, parked) in [
            (FRAMEBUFFER_WIDTH, off_width),
            (FRAMEBUFFER_HEIGHT, off_height),
        ] {
            builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
            builder.emit(abi::store_u32(
                abi::SCRATCH[0],
                abi::stack_pointer(),
                at.info + field,
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
            at.info,
            off_out,
            GRAPHICS_OFFSET_VULKAN_SC_FRAMEBUFFERS + i * 8,
            declined,
        )?;
        builder.emit(abi::label(&skip));
    }
    for (state, parked) in [
        (GRAPHICS_OFFSET_VULKAN_SC_W, off_width),
        (GRAPHICS_OFFSET_VULKAN_SC_H, off_height),
    ] {
        builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), parked));
        emit_state_store(builder, off_state, state, abi::SCRATCH[0]);
    }
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_STALE,
        abi::SCRATCH[0],
    );

    // --- acquire -----------------------------------------------------------------
    builder.emit(abi::label(&have_swapchain));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkAcquireNextImageKHR",
        off_handle,
        off_fn,
        declined,
    )?;
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.index,
    ));
    park_state(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SWAPCHAIN,
        at.swapchain,
    );
    park_state(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE,
        at.sem_acquire,
    );
    emit_int_arg(builder, platform, 4, "0"); // fence
    emit_addr_arg(builder, platform, 5, at.index);
    emit_int_arg_slot(builder, platform, 3, at.sem_acquire);
    emit_int_arg(builder, platform, 2, ACQUIRE_TIMEOUT_NS);
    emit_int_arg_slot(builder, platform, 1, at.swapchain);
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::c_arg(0),
    );
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&acquired));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        RESULT_SUBOPTIMAL,
    ));
    builder.emit(abi::compare_registers(abi::c_return(0), abi::SCRATCH[0]));
    builder.emit(abi::branch_ne(&stale_acquire));
    // Acquired, but rebuild before the next frame.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_STALE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::branch(&acquired));
    // Out of date, timed out, or lost: nothing was acquired; rebuild next frame and
    // render this one down the readback path.
    builder.emit(abi::label(&stale_acquire));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_STALE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::branch(declined));

    builder.emit(abi::label(&acquired));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.acquired,
    ));
    emit_park_framebuffer(builder, off_state, at);
    let done = builder.label("vk_sc_ready");
    builder.emit(abi::branch(&done));

    // A device or surface that can never present: release what was made for it, and
    // every later frame declines at once.
    builder.emit(abi::label(&failed));
    emit_teardown(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        at,
        declined,
    )?;
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_FAILED,
        abi::SCRATCH[0],
    );
    builder.emit(abi::branch(declined));
    builder.emit(abi::label(&done));
    Ok(())
}
