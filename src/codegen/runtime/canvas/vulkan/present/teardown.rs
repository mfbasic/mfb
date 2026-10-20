//! plan-176-V: the swapchain teardown, abandon and destroy. The shared offsets, `PresentSlots`
//! and the present entry points live in `runtime/canvas/vulkan/present/mod.rs`.

use super::*;

/// Destroy everything built on the canvas window — framebuffers, views, the swapchain
/// and the surface — once the device is idle. Each handle that is 0 is skipped
/// (`vkDestroySurfaceKHR` accepts a null surface), so it serves the first build too.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_teardown(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    at: &PresentSlots,
    declined: &str,
) -> Result<(), String> {
    emit_device_call_1(
        builder,
        platform,
        platform_imports,
        "vkDeviceWaitIdle",
        off_handle,
        off_fn,
        off_state,
        declined,
    )?;
    for i in 0..VULKAN_MAX_SWAPCHAIN_IMAGES {
        for (name, slot) in [
            (
                "vkDestroyFramebuffer",
                GRAPHICS_OFFSET_VULKAN_SC_FRAMEBUFFERS + i * 8,
            ),
            (
                "vkDestroyImageView",
                GRAPHICS_OFFSET_VULKAN_SC_VIEWS + i * 8,
            ),
        ] {
            let skip = builder.label("vk_sc_destroy_skip");
            emit_state_load(builder, off_state, slot, abi::SCRATCH[0]);
            builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
            builder.emit(abi::branch_eq(&skip));
            emit_destroy(
                builder,
                platform,
                platform_imports,
                off_handle,
                off_fn,
                off_state,
                name,
                GRAPHICS_OFFSET_VULKAN_DEVICE,
                slot,
                declined,
            )?;
            builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
            emit_state_store(builder, off_state, slot, abi::SCRATCH[0]);
            builder.emit(abi::label(&skip));
        }
    }
    let skip = builder.label("vk_sc_destroy_chain_skip");
    park_state(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SWAPCHAIN,
        at.swapchain,
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.swapchain,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&skip));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkDestroySwapchainKHR",
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
    builder.emit(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        at.swapchain,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::label(&skip));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SWAPCHAIN,
        abi::SCRATCH[0],
    );
    emit_destroy(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        "vkDestroySurfaceKHR",
        GRAPHICS_OFFSET_VULKAN_INSTANCE,
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        declined,
    )?;
    for slot in [
        GRAPHICS_OFFSET_VULKAN_SURFACE,
        GRAPHICS_OFFSET_VULKAN_SURFACE_HWND,
    ] {
        builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        emit_state_store(builder, off_state, slot, abi::SCRATCH[0]);
    }
    Ok(())
}

/// The exit for a frame that declined after acquiring an image: the acquire
/// semaphore is (or will be) signalled and nothing will wait on it, so both semaphores
/// are dropped once the device is idle and made afresh next frame, and the swapchain is
/// marked stale so the rebuild releases the image that was never presented. A frame
/// that declined before acquiring falls straight through.
#[allow(clippy::too_many_arguments)]
pub(in crate::codegen::runtime::canvas::vulkan) fn emit_present_abandon(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    at: &PresentSlots,
) -> Result<(), String> {
    let clean = builder.label("vk_sc_abandon_clean");
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.acquired,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&clean));
    // bug-765: the image is given up with the frame, whichever pass gave up.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_HELD,
        abi::SCRATCH[0],
    );
    emit_device_call_1(
        builder,
        platform,
        platform_imports,
        "vkDeviceWaitIdle",
        off_handle,
        off_fn,
        off_state,
        &clean,
    )?;
    for slot in [
        GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE,
        GRAPHICS_OFFSET_VULKAN_SEM_RENDER,
    ] {
        emit_destroy(
            builder,
            platform,
            platform_imports,
            off_handle,
            off_fn,
            off_state,
            "vkDestroySemaphore",
            GRAPHICS_OFFSET_VULKAN_DEVICE,
            slot,
            &clean,
        )?;
        builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
        emit_state_store(builder, off_state, slot, abi::SCRATCH[0]);
    }
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_STALE,
        abi::SCRATCH[0],
    );
    builder.emit(abi::label(&clean));
    Ok(())
}

/// `vkDestroyX(owner, object, NULL)` for the owner and object state words.
#[allow(clippy::too_many_arguments)]
fn emit_destroy(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    name: &str,
    owner: usize,
    object: usize,
    declined: &str,
) -> Result<(), String> {
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        name,
        off_handle,
        off_fn,
        declined,
    )?;
    emit_state_load(builder, off_state, owner, abi::c_arg(0));
    emit_state_load(builder, off_state, object, abi::c_arg(1));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    emit_call_fn(builder, off_fn);
    Ok(())
}
