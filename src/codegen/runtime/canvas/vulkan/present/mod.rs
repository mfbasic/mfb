//! bug-688 V6: present a Vulkan frame straight to the window through a swapchain.
//!
//! The readback path renders offscreen, copies the image into a host buffer, swizzles
//! it into a CPU surface and hands that to the window thread to blit. Measured on a
//! real Intel UHD 630 (box 206, 95 items, 1920x1080): the GPU's share of that frame is
//! ~2.4 ms and the readback path's CPU work is ~36 ms — over the 60 Hz budget on its
//! own. This is the Vulkan twin of Metal's direct present (bug-686 Phase 4): the frame
//! is recorded into a swapchain image and presented, with no copy, no swizzle, no CPU
//! surface and no blit.
//!
//! **Windows only**, through `VK_KHR_win32_surface` on the canvas's child window
//! (`_mfb_winapp_canvas_child`). Every other target, a headless run (no window, handle
//! 0), a driver without the extensions, a surface that cannot take the attachment
//! format, and a window whose surface size differs from the canvas size all answer
//! FALSE, and the caller renders that frame down the readback path — so every headless
//! oracle test compares exactly the pixels it always did.
//!
//! **Why a child window, not the top-level one.** Once a swapchain presents to a
//! window, the driver's flip presentation owns what that window shows: GDI painting of
//! it — the readback frame's `SetDIBitsToDevice`, and the transcript EDIT when the
//! program returns to `Console` — no longer reaches the screen. Measured on box 206
//! with a Desktop Duplication capture (`tools/win-screen-capture`; a GDI screen grab
//! cannot see swapchain content reliably): with the swapchain on the top-level window,
//! `app::setMode(Console)` left the last canvas frame on screen over the transcript. The swapchain objects belong to the graphics
//! thread, which nothing wakes on a mode change, so they cannot be released there.
//! Instead the frame presents to a child window the UI thread shows and hides: a
//! directly presented frame posts `WM_APP_CANVAS_SHOW` (shown only while still in
//! canvas mode), and a readback frame's `WM_APP_BLIT` and every reconcile out of
//! canvas mode hide it. Both messages are posted in frame order, so whichever path the
//! last frame took is the one on screen.
//!
//! The offscreen render pass's pipelines are reused: a render pass whose only
//! difference is the final layout (`PRESENT_SRC_KHR`) and a subpass dependency is
//! *compatible* with it, so no pipeline is rebuilt.

use super::*;
use crate::codegen::runtime::canvas::{
    GRAPHICS_OFFSET_VULKAN_SC_COUNT, GRAPHICS_OFFSET_VULKAN_SC_FAILED,
    GRAPHICS_OFFSET_VULKAN_SC_FRAMEBUFFERS, GRAPHICS_OFFSET_VULKAN_SC_H,
    GRAPHICS_OFFSET_VULKAN_SC_HELD, GRAPHICS_OFFSET_VULKAN_SC_IMAGES,
    GRAPHICS_OFFSET_VULKAN_SC_STALE, GRAPHICS_OFFSET_VULKAN_SC_VIEWS, GRAPHICS_OFFSET_VULKAN_SC_W,
    GRAPHICS_OFFSET_VULKAN_SEM_ACQUIRE, GRAPHICS_OFFSET_VULKAN_SEM_RENDER,
    GRAPHICS_OFFSET_VULKAN_SURFACE, GRAPHICS_OFFSET_VULKAN_SURFACE_HWND,
    GRAPHICS_OFFSET_VULKAN_SWAPCHAIN, GRAPHICS_OFFSET_VULKAN_WSI, VULKAN_MAX_SWAPCHAIN_IMAGES,
};

mod render_pass;
mod submit;
mod swapchain;
mod teardown;
mod wsi;

pub(super) use self::{
    submit::emit_submit_and_present,
    swapchain::emit_ensure_swapchain,
    teardown::emit_present_abandon,
    wsi::{emit_wsi_create_device, emit_wsi_create_instance},
};
use render_pass::emit_present_render_pass;
use teardown::emit_teardown;

/// The window handle the Windows app publishes while a canvas window is on screen.
const CANVAS_HWND_SYMBOL: &str = "_mfb_winapp_canvas_hwnd";
/// The child window the swapchain presents to, sized to the client area by the UI
/// thread; 0 until canvas mode is first entered. Owned by the Windows app.
pub(crate) const CANVAS_CHILD_SYMBOL: &str = "_mfb_winapp_canvas_child";
/// `WM_APP + 4`: "a frame was presented to the child — show it if still in canvas
/// mode". Posted to the top-level window after every direct present.
pub(crate) const WM_APP_CANVAS_SHOW: &str = "32772";

/// The extension names, as C strings the create-info arrays point at.
pub(super) const WSI_INSTANCE_EXTENSIONS: [&str; 2] = ["VK_KHR_surface", "VK_KHR_win32_surface"];
pub(super) const WSI_DEVICE_EXTENSIONS: [&str; 1] = ["VK_KHR_swapchain"];

pub(super) fn extension_name_symbol(name: &str) -> String {
    format!("_mfb_canvas_vk_ext_{name}")
}

/// The extension-name C strings, emitted beside the entry-point names.
pub(super) fn wsi_data_objects() -> Vec<CodeDataObject> {
    WSI_INSTANCE_EXTENSIONS
        .iter()
        .chain(WSI_DEVICE_EXTENSIONS.iter())
        .map(|name| CodeDataObject {
            symbol: extension_name_symbol(name),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: name.len() + 1,
            value: hex_encode_cstring(name),
        })
        .collect()
}

// ---------------------------------------------------------------------------------
// Layouts (LP64 and LLP64 agree: every handle is 8 bytes, every enum/flags 4).
// ---------------------------------------------------------------------------------

const INSTANCE_INFO_EXTENSION_COUNT: usize = 48;
const INSTANCE_INFO_EXTENSIONS: usize = 56;
const DEVICE_INFO_EXTENSION_COUNT: usize = 48;
const DEVICE_INFO_EXTENSIONS: usize = 56;

/// `VkWin32SurfaceCreateInfoKHR`, 40 bytes.
const WIN32_SURFACE_INFO_SIZE: usize = 40;
const WIN32_SURFACE_HINSTANCE: usize = 24;
const WIN32_SURFACE_HWND: usize = 32;
const ST_WIN32_SURFACE_CREATE_INFO_KHR: &str = "1000009000";

/// `VkSurfaceCapabilitiesKHR`, 52 bytes (reserved 56).
const CAPS_SIZE: usize = 56;
const CAPS_MIN_IMAGES: usize = 0;
const CAPS_MAX_IMAGES: usize = 4;
const CAPS_CURRENT_WIDTH: usize = 8;
const CAPS_CURRENT_HEIGHT: usize = 12;
const CAPS_CURRENT_TRANSFORM: usize = 40;
const CAPS_COMPOSITE_ALPHA: usize = 44;

/// `VkSurfaceFormatKHR`, 8 bytes; the present path asks for up to this many.
const SURFACE_FORMAT_STRIDE: usize = 8;
const MAX_SURFACE_FORMATS: usize = 32;

/// `VkSwapchainCreateInfoKHR`, 104 bytes.
const SWAPCHAIN_INFO_SIZE: usize = 104;
const SWAPCHAIN_SURFACE: usize = 24;
const SWAPCHAIN_MIN_IMAGES: usize = 32;
const SWAPCHAIN_FORMAT: usize = 36;
const SWAPCHAIN_COLOR_SPACE: usize = 40;
const SWAPCHAIN_WIDTH: usize = 44;
const SWAPCHAIN_HEIGHT: usize = 48;
const SWAPCHAIN_LAYERS: usize = 52;
const SWAPCHAIN_USAGE: usize = 56;
const SWAPCHAIN_PRE_TRANSFORM: usize = 80;
const SWAPCHAIN_COMPOSITE_ALPHA: usize = 84;
const SWAPCHAIN_PRESENT_MODE: usize = 88;
const SWAPCHAIN_CLIPPED: usize = 92;
const ST_SWAPCHAIN_CREATE_INFO_KHR: &str = "1000001000";

/// `VkSemaphoreCreateInfo`, 24 bytes.
const SEMAPHORE_INFO_SIZE: usize = 24;
const ST_SEMAPHORE_CREATE_INFO: &str = "9";

/// `VkSubmitInfo`'s semaphore members (the rest are in `vulkan/mod.rs`).
const SUBMIT_WAIT_COUNT: usize = 16;
const SUBMIT_WAIT_SEMAPHORES: usize = 24;
const SUBMIT_WAIT_STAGES: usize = 32;
const SUBMIT_SIGNAL_COUNT: usize = 56;
const SUBMIT_SIGNAL_SEMAPHORES: usize = 64;

/// `VkPresentInfoKHR`, 64 bytes.
const PRESENT_INFO_SIZE: usize = 64;
const PRESENT_WAIT_COUNT: usize = 16;
const PRESENT_WAIT_SEMAPHORES: usize = 24;
const PRESENT_SWAPCHAIN_COUNT: usize = 32;
const PRESENT_SWAPCHAINS: usize = 40;
const PRESENT_IMAGE_INDICES: usize = 48;
const ST_PRESENT_INFO_KHR: &str = "1000001001";

/// `VkSubpassDependency`, 28 bytes (reserved 32), and the render-pass members that
/// point at it.
const DEPENDENCY_SIZE: usize = 32;
const DEPENDENCY_SRC_SUBPASS: usize = 0;
const DEPENDENCY_DST_SUBPASS: usize = 4;
const DEPENDENCY_SRC_STAGE: usize = 8;
const DEPENDENCY_DST_STAGE: usize = 12;
const DEPENDENCY_DST_ACCESS: usize = 20;
const RENDER_PASS_DEPENDENCY_COUNT: usize = 48;
const RENDER_PASS_DEPENDENCIES: usize = 56;

const SUBPASS_EXTERNAL: &str = "4294967295";
/// `VK_PIPELINE_STAGE_COLOR_ATTACHMENT_OUTPUT_BIT`.
const STAGE_COLOR_ATTACHMENT_OUTPUT: &str = "1024";
/// `VK_ACCESS_COLOR_ATTACHMENT_WRITE_BIT`.
const ACCESS_COLOR_ATTACHMENT_WRITE: &str = "256";
/// `VK_IMAGE_LAYOUT_PRESENT_SRC_KHR`.
pub(super) const IMAGE_LAYOUT_PRESENT_SRC: &str = "1000001002";
/// `VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT`.
const USAGE_COLOR_ATTACHMENT: &str = "16";
/// `VK_COLOR_SPACE_SRGB_NONLINEAR_KHR`.
const COLOR_SPACE_SRGB_NONLINEAR: &str = "0";
/// `VK_PRESENT_MODE_FIFO_KHR` — the one mode every implementation must offer, and
/// the display-paced one.
const PRESENT_MODE_FIFO: &str = "2";
/// `VK_SUBOPTIMAL_KHR`: presented (or acquired) but the swapchain should be rebuilt.
const RESULT_SUBOPTIMAL: &str = "1000001003";
/// `0xFFFFFFFF` in `currentExtent`: the surface size follows the swapchain.
const EXTENT_UNDEFINED: &str = "4294967295";
/// An acquire waits this long at most (ns) before the frame falls back to readback,
/// so a minimised or occluded window can never hang the graphics thread.
const ACQUIRE_TIMEOUT_NS: &str = "100000000";

/// Is the swapchain path compiled for this platform at all?
pub(crate) fn has_present_path(platform: &dyn CodegenPlatform) -> bool {
    matches!(platform.family(), PlatformFamily::Windows)
}

/// The stack slots the present path shares with `emit_vulkan_draw_scene`.
pub(super) struct PresentSlots {
    /// The acquired image's index (u32 in a u64 slot).
    pub index: usize,
    /// That image's framebuffer.
    pub framebuffer: usize,
    /// 1 once the frame was presented, else 0 — the function's answer.
    pub presented: usize,
    /// 1 while an image is acquired and not yet handed to the queue.
    acquired: usize,
    swapchain: usize,
    sem_acquire: usize,
    sem_render: usize,
    wait_stage: usize,
    hwnd: usize,
    scratch: usize,
    caps: usize,
    formats: usize,
    count: usize,
    info: usize,
    view_handle: usize,
    submit: usize,
    present: usize,
}

impl PresentSlots {
    pub(super) fn allocate(builder: &mut CodeBuilder) -> Self {
        Self {
            index: builder.allocate_stack_object("vk_sc_index", 8),
            framebuffer: builder.allocate_stack_object("vk_sc_framebuffer", 8),
            presented: builder.allocate_stack_object("vk_sc_presented", 8),
            acquired: builder.allocate_stack_object("vk_sc_acquired", 8),
            swapchain: builder.allocate_stack_object("vk_sc_handle", 8),
            sem_acquire: builder.allocate_stack_object("vk_sc_sem_acquire", 8),
            sem_render: builder.allocate_stack_object("vk_sc_sem_render", 8),
            wait_stage: builder.allocate_stack_object("vk_sc_wait_stage", 8),
            hwnd: builder.allocate_stack_object("vk_sc_hwnd", 8),
            scratch: builder.allocate_stack_object("vk_sc_scratch", 8),
            caps: builder.allocate_stack_object("vk_sc_caps", CAPS_SIZE),
            formats: builder.allocate_stack_object(
                "vk_sc_formats",
                MAX_SURFACE_FORMATS * SURFACE_FORMAT_STRIDE,
            ),
            count: builder.allocate_stack_object("vk_sc_count", 8),
            info: builder.allocate_stack_object("vk_sc_info", SWAPCHAIN_INFO_SIZE),
            view_handle: builder.allocate_stack_object("vk_sc_view_handle", 8),
            submit: builder.allocate_stack_object("vk_sc_submit", SUBMIT_INFO_SIZE),
            present: builder.allocate_stack_object("vk_sc_present", PRESENT_INFO_SIZE),
        }
    }
}

/// Start a frame answering FALSE with nothing acquired. Runs before anything can
/// branch to the function's exits, which read both.
pub(super) fn emit_present_begin(builder: &mut CodeBuilder, at: &PresentSlots) {
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for slot in [at.presented, at.acquired] {
        builder.emit(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
    }
}

/// bug-765: take up the swapchain image this frame's first pass acquired and left
/// held (`SC_HELD`): its index and framebuffer, and the mark that an image is out of
/// the swapchain and not yet handed to the queue. Nothing held leaves the frame as
/// `emit_present_begin` started it, and `emit_require_acquired` then gives it up.
pub(super) fn emit_present_resume(builder: &mut CodeBuilder, off_state: usize, at: &PresentSlots) {
    let none = builder.label("vk_sc_none_held");
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SC_HELD,
        abi::SCRATCH[1],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], "0"));
    builder.emit(abi::branch_eq(&none));
    builder.emit(abi::subtract_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.index,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.acquired,
    ));
    emit_park_framebuffer(builder, off_state, at);
    builder.emit(abi::label(&none));
}

/// Branch to `declined` unless this frame holds an acquired image.
pub(super) fn emit_require_acquired(builder: &mut CodeBuilder, at: &PresentSlots, declined: &str) {
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.acquired,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(declined));
}

/// `at.framebuffer = SC_FRAMEBUFFERS[at.index]`.
fn emit_park_framebuffer(builder: &mut CodeBuilder, off_state: usize, at: &PresentSlots) {
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.index,
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_state,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        GRAPHICS_OFFSET_VULKAN_SC_FRAMEBUFFERS,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.framebuffer,
    ));
}

/// Load the state word at `offset` into the stack slot `slot`.
fn park_state(builder: &mut CodeBuilder, off_state: usize, offset: usize, slot: usize) {
    emit_state_load(builder, off_state, offset, abi::SCRATCH[0]);
    builder.emit(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::runtime::canvas::vulkan::tests::assert_layout;

    /// The swapchain path's struct tables, checked like the renderer's
    /// (`layout_tables_are_self_consistent`).
    #[test]
    fn present_layout_tables_are_self_consistent() {
        assert_layout(
            "VkInstanceCreateInfo (extensions)",
            INSTANCE_INFO_SIZE,
            8,
            &[
                ("pApplicationInfo", INSTANCE_INFO_APP_INFO, 8),
                ("enabledExtensionCount", INSTANCE_INFO_EXTENSION_COUNT, 4),
                ("ppEnabledExtensionNames", INSTANCE_INFO_EXTENSIONS, 8),
            ],
        );
        assert_layout(
            "VkDeviceCreateInfo (extensions)",
            DEVICE_INFO_SIZE,
            8,
            &[
                ("pQueueCreateInfos", DEVICE_INFO_QUEUE_INFOS, 8),
                ("enabledExtensionCount", DEVICE_INFO_EXTENSION_COUNT, 4),
                ("ppEnabledExtensionNames", DEVICE_INFO_EXTENSIONS, 8),
            ],
        );
        assert_layout(
            "VkWin32SurfaceCreateInfoKHR",
            WIN32_SURFACE_INFO_SIZE,
            8,
            &[
                ("hinstance", WIN32_SURFACE_HINSTANCE, 8),
                ("hwnd", WIN32_SURFACE_HWND, 8),
            ],
        );
        assert_layout(
            "VkSurfaceCapabilitiesKHR",
            CAPS_SIZE,
            4,
            &[
                ("minImageCount", CAPS_MIN_IMAGES, 4),
                ("maxImageCount", CAPS_MAX_IMAGES, 4),
                ("currentExtent.width", CAPS_CURRENT_WIDTH, 4),
                ("currentExtent.height", CAPS_CURRENT_HEIGHT, 4),
                ("currentTransform", CAPS_CURRENT_TRANSFORM, 4),
                ("supportedCompositeAlpha", CAPS_COMPOSITE_ALPHA, 4),
            ],
        );
        assert_layout(
            "VkSwapchainCreateInfoKHR",
            SWAPCHAIN_INFO_SIZE,
            8,
            &[
                ("surface", SWAPCHAIN_SURFACE, 8),
                ("minImageCount", SWAPCHAIN_MIN_IMAGES, 4),
                ("imageFormat", SWAPCHAIN_FORMAT, 4),
                ("imageColorSpace", SWAPCHAIN_COLOR_SPACE, 4),
                ("imageExtent.width", SWAPCHAIN_WIDTH, 4),
                ("imageExtent.height", SWAPCHAIN_HEIGHT, 4),
                ("imageArrayLayers", SWAPCHAIN_LAYERS, 4),
                ("imageUsage", SWAPCHAIN_USAGE, 4),
                ("preTransform", SWAPCHAIN_PRE_TRANSFORM, 4),
                ("compositeAlpha", SWAPCHAIN_COMPOSITE_ALPHA, 4),
                ("presentMode", SWAPCHAIN_PRESENT_MODE, 4),
                ("clipped", SWAPCHAIN_CLIPPED, 4),
            ],
        );
        assert_layout("VkSemaphoreCreateInfo", SEMAPHORE_INFO_SIZE, 8, &[]);
        assert_layout(
            "VkSubmitInfo (semaphores)",
            SUBMIT_INFO_SIZE,
            8,
            &[
                ("waitSemaphoreCount", SUBMIT_WAIT_COUNT, 4),
                ("pWaitSemaphores", SUBMIT_WAIT_SEMAPHORES, 8),
                ("pWaitDstStageMask", SUBMIT_WAIT_STAGES, 8),
                ("commandBufferCount", SUBMIT_COMMAND_COUNT, 4),
                ("pCommandBuffers", SUBMIT_COMMANDS, 8),
                ("signalSemaphoreCount", SUBMIT_SIGNAL_COUNT, 4),
                ("pSignalSemaphores", SUBMIT_SIGNAL_SEMAPHORES, 8),
            ],
        );
        assert_layout(
            "VkPresentInfoKHR",
            PRESENT_INFO_SIZE,
            8,
            &[
                ("waitSemaphoreCount", PRESENT_WAIT_COUNT, 4),
                ("pWaitSemaphores", PRESENT_WAIT_SEMAPHORES, 8),
                ("swapchainCount", PRESENT_SWAPCHAIN_COUNT, 4),
                ("pSwapchains", PRESENT_SWAPCHAINS, 8),
                ("pImageIndices", PRESENT_IMAGE_INDICES, 8),
            ],
        );
        assert_layout(
            "VkSubpassDependency",
            DEPENDENCY_SIZE,
            4,
            &[
                ("srcSubpass", DEPENDENCY_SRC_SUBPASS, 4),
                ("dstSubpass", DEPENDENCY_DST_SUBPASS, 4),
                ("srcStageMask", DEPENDENCY_SRC_STAGE, 4),
                ("dstStageMask", DEPENDENCY_DST_STAGE, 4),
                ("dstAccessMask", DEPENDENCY_DST_ACCESS, 4),
            ],
        );
        assert_layout(
            "VkRenderPassCreateInfo (dependencies)",
            RENDER_PASS_INFO_SIZE,
            8,
            &[
                ("pSubpasses", RENDER_PASS_SUBPASSES, 8),
                ("dependencyCount", RENDER_PASS_DEPENDENCY_COUNT, 4),
                ("pDependencies", RENDER_PASS_DEPENDENCIES, 8),
            ],
        );
    }

    /// The swapchain path is Windows-only, and every entry point and extension name it
    /// resolves has a data object — a missing one is a link failure, but only on the
    /// one target that emits the reference.
    #[test]
    fn the_present_path_is_windows_only_and_fully_named() {
        use crate::target::linux_common::code::Platform as LinuxPlatform;
        let linux = LinuxPlatform::for_test(
            crate::target::linux_x86_64::code::X86_64,
            crate::os::linux::flavor::LinuxFlavor::Glibc,
        );
        let macos = crate::target::macos_aarch64::code::Platform;
        let windows = crate::target::win_x86_64::code::Platform;
        let platforms: [(&str, &dyn CodegenPlatform, bool); 3] = [
            ("linux", &linux, false),
            ("macos", &macos, false),
            ("windows", &windows, true),
        ];
        for (family, platform, expected) in platforms {
            assert_eq!(has_present_path(platform), expected, "{family}");
        }
        for name in [
            "vkCreateWin32SurfaceKHR",
            "vkGetPhysicalDeviceSurfaceSupportKHR",
            "vkGetPhysicalDeviceSurfaceCapabilitiesKHR",
            "vkGetPhysicalDeviceSurfaceFormatsKHR",
            "vkCreateSwapchainKHR",
            "vkGetSwapchainImagesKHR",
            "vkAcquireNextImageKHR",
            "vkQueuePresentKHR",
            "vkDestroySwapchainKHR",
            "vkDestroySurfaceKHR",
            "vkCreateSemaphore",
            "vkDestroySemaphore",
        ] {
            assert!(
                VK_PROBE_ENTRY_POINTS.contains(&name),
                "{name} is not resolvable"
            );
        }
        let names: Vec<String> = wsi_data_objects().into_iter().map(|o| o.symbol).collect();
        for ext in WSI_INSTANCE_EXTENSIONS
            .iter()
            .chain(WSI_DEVICE_EXTENSIONS.iter())
        {
            assert!(
                names.contains(&extension_name_symbol(ext)),
                "{ext} has no C string"
            );
        }
    }
}
