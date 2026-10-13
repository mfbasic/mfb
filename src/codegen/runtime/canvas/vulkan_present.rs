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

/// `VkSubmitInfo`'s semaphore members (the rest are in `vulkan.rs`).
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

/// Point `n` extension-name slots at `names`, starting at `sp + array`.
fn emit_name_array(builder: &mut CodeBuilder, array: usize, names: &[&str]) {
    let symbol = builder.current_symbol.clone();
    for (i, name) in names.iter().enumerate() {
        emit_data_address(
            &symbol,
            abi::SCRATCH[0],
            &extension_name_symbol(name),
            &mut builder.instructions,
            &mut builder.relocations,
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            array + i * 8,
        ));
    }
}

/// `vkCreateInstance`, asking for the window-system extensions first and retrying
/// without them if the driver refuses (`VK_ERROR_EXTENSION_NOT_PRESENT`): a machine
/// that cannot present still renders, through readback. `vkCreateInstance` has been
/// resolved into `off_fn`; the answer is in `c_return(0)` like the plain call's, and
/// `…_VULKAN_WSI` records whether the extensions were taken.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_wsi_create_instance(
    builder: &mut CodeBuilder,
    off_instance_info: usize,
    off_out: usize,
    off_fn: usize,
    off_state: usize,
) {
    let plain = builder.label("vk_wsi_instance_plain");
    let done = builder.label("vk_wsi_instance_done");
    let off_names = builder.allocate_stack_object("vk_wsi_instance_names", 16);
    emit_name_array(builder, off_names, &WSI_INSTANCE_EXTENSIONS);
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "2"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_instance_info + INSTANCE_INFO_EXTENSION_COUNT,
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_names,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_instance_info + INSTANCE_INFO_EXTENSIONS,
    ));
    emit_create_instance_call(builder, off_instance_info, off_out, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&plain));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_WSI,
        abi::SCRATCH[0],
    );
    builder.emit(abi::move_immediate(abi::c_return(0), "Integer", "0"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&plain));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_instance_info + INSTANCE_INFO_EXTENSION_COUNT,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_instance_info + INSTANCE_INFO_EXTENSIONS,
    ));
    emit_create_instance_call(builder, off_instance_info, off_out, off_fn);
    builder.emit(abi::label(&done));
}

fn emit_create_instance_call(
    builder: &mut CodeBuilder,
    off_instance_info: usize,
    off_out: usize,
    off_fn: usize,
) {
    builder.emit(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_instance_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
}

/// `vkCreateDevice`, asking for `VK_KHR_swapchain` when the instance took the
/// window-system extensions and retrying without it (clearing `…_VULKAN_WSI`) if the
/// device refuses. `vkCreateDevice` has been resolved into `off_fn`; the answer is in
/// `c_return(0)`.
pub(super) fn emit_wsi_create_device(
    builder: &mut CodeBuilder,
    off_device_info: usize,
    off_out: usize,
    off_fn: usize,
    off_state: usize,
) {
    let plain = builder.label("vk_wsi_device_plain");
    let call_plain = builder.label("vk_wsi_device_call_plain");
    let done = builder.label("vk_wsi_device_done");
    let off_names = builder.allocate_stack_object("vk_wsi_device_names", 8);
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_WSI,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "1"));
    builder.emit(abi::branch_ne(&call_plain));
    emit_name_array(builder, off_names, &WSI_DEVICE_EXTENSIONS);
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_device_info + DEVICE_INFO_EXTENSION_COUNT,
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_names,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_device_info + DEVICE_INFO_EXTENSIONS,
    ));
    emit_create_device_call(builder, off_device_info, off_out, off_fn, off_state);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&plain));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&plain));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_WSI,
        abi::SCRATCH[0],
    );
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_device_info + DEVICE_INFO_EXTENSION_COUNT,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_device_info + DEVICE_INFO_EXTENSIONS,
    ));
    builder.emit(abi::label(&call_plain));
    emit_create_device_call(builder, off_device_info, off_out, off_fn, off_state);
    builder.emit(abi::label(&done));
}

fn emit_create_device_call(
    builder: &mut CodeBuilder,
    off_device_info: usize,
    off_out: usize,
    off_fn: usize,
    off_state: usize,
) {
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_device_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
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

/// Make sure a swapchain for the canvas window exists at `width` x `height`, and
/// acquire its next image: on return the image's index and framebuffer are in
/// `at.index` / `at.framebuffer` and the acquire semaphore will signal. Branches to
/// `declined` whenever this frame cannot be presented directly — the caller renders it
/// down the readback path instead.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_ensure_swapchain(
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

/// The present render pass: the offscreen pass's single sRGB attachment, cleared and
/// stored, ending in `PRESENT_SRC_KHR`, with the external dependency that orders the
/// attachment write after the acquire semaphore's wait. Built once.
#[allow(clippy::too_many_arguments)]
fn emit_present_render_pass(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    at: &PresentSlots,
    failed: &str,
) -> Result<(), String> {
    let have = builder.label("vk_sc_have_pass");
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PRESENT_PASS,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(&have));
    let off_attachment = builder.allocate_stack_object("vk_sc_attachment", ATTACHMENT_SIZE + 4);
    let off_ref = builder.allocate_stack_object("vk_sc_attachment_ref", ATTACHMENT_REF_SIZE);
    let off_subpass = builder.allocate_stack_object("vk_sc_subpass", SUBPASS_SIZE);
    let off_dependency = builder.allocate_stack_object("vk_sc_dependency", DEPENDENCY_SIZE);
    emit_struct(
        builder,
        off_attachment,
        ATTACHMENT_SIZE + 4,
        &[
            (ATTACHMENT_FORMAT, Field::U32(FORMAT_B8G8R8A8_SRGB)),
            (ATTACHMENT_SAMPLES, Field::U32(SAMPLE_COUNT_1)),
            (ATTACHMENT_LOAD_OP, Field::U32(ATTACHMENT_LOAD_OP_CLEAR)),
            (ATTACHMENT_STORE_OP, Field::U32(ATTACHMENT_STORE_OP_STORE)),
            (
                ATTACHMENT_INITIAL_LAYOUT,
                Field::U32(IMAGE_LAYOUT_UNDEFINED),
            ),
            (
                ATTACHMENT_FINAL_LAYOUT,
                Field::U32(IMAGE_LAYOUT_PRESENT_SRC),
            ),
        ],
    );
    emit_struct(
        builder,
        off_ref,
        ATTACHMENT_REF_SIZE,
        &[
            (ATTACHMENT_REF_INDEX, Field::U32("0")),
            (
                ATTACHMENT_REF_LAYOUT,
                Field::U32(IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL),
            ),
        ],
    );
    emit_struct(
        builder,
        off_subpass,
        SUBPASS_SIZE,
        &[
            (SUBPASS_BIND_POINT, Field::U32(PIPELINE_BIND_POINT_GRAPHICS)),
            (SUBPASS_COLOR_COUNT, Field::U32("1")),
            (SUBPASS_COLOR_ATTACHMENTS, Field::Addr(off_ref)),
        ],
    );
    emit_struct(
        builder,
        off_dependency,
        DEPENDENCY_SIZE,
        &[
            (DEPENDENCY_SRC_SUBPASS, Field::U32(SUBPASS_EXTERNAL)),
            (DEPENDENCY_DST_SUBPASS, Field::U32("0")),
            (
                DEPENDENCY_SRC_STAGE,
                Field::U32(STAGE_COLOR_ATTACHMENT_OUTPUT),
            ),
            (
                DEPENDENCY_DST_STAGE,
                Field::U32(STAGE_COLOR_ATTACHMENT_OUTPUT),
            ),
            (
                DEPENDENCY_DST_ACCESS,
                Field::U32(ACCESS_COLOR_ATTACHMENT_WRITE),
            ),
        ],
    );
    emit_struct(
        builder,
        at.info,
        RENDER_PASS_INFO_SIZE,
        &[
            (RENDER_PASS_STYPE, Field::U32(ST_RENDER_PASS_CREATE_INFO)),
            (RENDER_PASS_ATTACHMENT_COUNT, Field::U32("1")),
            (RENDER_PASS_ATTACHMENTS, Field::Addr(off_attachment)),
            (RENDER_PASS_SUBPASS_COUNT, Field::U32("1")),
            (RENDER_PASS_SUBPASSES, Field::Addr(off_subpass)),
            (RENDER_PASS_DEPENDENCY_COUNT, Field::U32("1")),
            (RENDER_PASS_DEPENDENCIES, Field::Addr(off_dependency)),
        ],
    );
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateRenderPass",
        off_handle,
        off_fn,
        off_state,
        at.info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_PRESENT_PASS,
        failed,
    )?;
    builder.emit(abi::label(&have));
    Ok(())
}

/// Destroy everything built on the canvas window — framebuffers, views, the swapchain
/// and the surface — once the device is idle. Each handle that is 0 is skipped
/// (`vkDestroySurfaceKHR` accepts a null surface), so it serves the first build too.
#[allow(clippy::too_many_arguments)]
fn emit_teardown(
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
pub(super) fn emit_present_abandon(
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

/// End the recording, submit it — waiting on the acquire semaphore, signalling the
/// render one — present the image, and wait for the queue. On success `at.presented`
/// is 1 and the canvas-direct flag is set so `WM_PAINT` leaves the window alone.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_submit_and_present(
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

#[cfg(test)]
mod tests {
    use super::*;

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
