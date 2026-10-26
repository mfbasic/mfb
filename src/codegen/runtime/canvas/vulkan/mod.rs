//! The Vulkan renderer's loader layer (plan-98-F Phase 1).
//!
//! Built in the order each piece can be *tested*, the same discipline plan-98-E used
//! for Metal: the loader and a device probe first, because they prove the whole
//! foreign-call path — the `dlopen` of `libvulkan.so.1`, the `vkGetInstanceProcAddr`
//! bootstrap, the C struct layouts, and calling through a resolved function pointer —
//! before any pipeline code is written on top. A device that cannot be enumerated is
//! a loader or ABI fault, and reading that off a one-call probe is far cheaper than
//! reading it off a blank window.
//!
//! ## Nothing is linked
//!
//! Vulkan arrives entirely through `dlopen`/`dlsym`, never a `DT_NEEDED` — the same
//! rule `audio` follows for `libasound.so.2` (plan-33-C §3.1) and for the same
//! reason: a binary that merely *mentions* canvas must still exec on a machine with
//! no Vulkan loader installed. Only `vkGetInstanceProcAddr` is resolved by `dlsym`;
//! every other entry point comes from it, which is the loader contract Vulkan
//! actually specifies rather than a shortcut.
//!
//! ## A function pointer goes in a fresh vreg, not a scratch token
//!
//! `builder.temporary_vreg()`, the way the `audio` backend's `emit_call_fnptr` does
//! it — never a fixed `abi::SCRATCH[k]`. With a fixed token the allocator is free to
//! give that physical register to something else across the call, and the `blr` then
//! jumps to whatever it holds: measured here as a PC of `0x8148535554415541`, which
//! is not an address at all but the bytes `push %r13; push %r12; push %rbp;
//! push %rbx; sub …` — a function prologue read as a target. That shape is worth
//! recognising, because it says "you called through garbage" and nothing else does.
//!
//! ## The struct layouts are written out, because nothing checks them
//!
//! There is no header to include and no compiler to agree with, so every offset below
//! is the C layout spelled out by hand. A wrong one does not fail to build — it
//! passes garbage to a driver, which is why each struct's field table names its
//! member and its offset, and why the probe is a separate testable step rather than
//! the first half of a pipeline.

use crate::codegen::engine::builder::CodeBuilder;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::link::thunk::emit_data_address;
// plan-163-B: the stroke-style words and the join-record header slots.
use crate::codegen::runtime::canvas::{
    emit_f32_bits, emit_f32_word_to_fixed, emit_fixed_to_f32_bits, emit_item_arc_flags,
    emit_item_float_field, emit_item_polygon_rule, emit_item_stroke_sign, emit_square_cap_reach,
    push_symbol_address, BAND_LIST_PER_EDGE, BAND_MAX, BAND_MIN_EDGES, BLEND_MODE_COUNT,
    CANVAS_DRAW_ENTRY_COUNT_SHIFT, CANVAS_DRAW_ENTRY_MODE, CANVAS_DRAW_ENTRY_SHIFT,
    CURVE_PIECE_SLOTS, EDGE_SLOTS, FIXED_POINT_SCALE, GEO_KIND_CURVEPOLYGON, GEO_KIND_PICTURE,
    GEO_KIND_POLYCURVE, GEO_KIND_POLYGON, GEO_KIND_POLYLINE, GEO_KIND_TEXT, GEO_KIND_TILE,
    GLYPH_META_H, GLYPH_META_SLOTS, GLYPH_META_START, GLYPH_META_W, GLYPH_META_X0, GLYPH_META_Y0,
    GLYPH_RUN_SLOTS, GRADIENT_STOP_WORDS, GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER,
    GRAPHICS_OFFSET_VULKAN_COMMAND_POOL, GRAPHICS_OFFSET_VULKAN_DESC_POOL,
    GRAPHICS_OFFSET_VULKAN_DESC_SET, GRAPHICS_OFFSET_VULKAN_DEVICE,
    GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER, GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
    GRAPHICS_OFFSET_VULKAN_EDGE_MEMORY, GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER,
    GRAPHICS_OFFSET_VULKAN_IMAGE, GRAPHICS_OFFSET_VULKAN_IMAGE_MEMORY,
    GRAPHICS_OFFSET_VULKAN_IMAGE_VIEW, GRAPHICS_OFFSET_VULKAN_INSTANCE,
    GRAPHICS_OFFSET_VULKAN_ITEM_BUFFER, GRAPHICS_OFFSET_VULKAN_ITEM_MAPPED,
    GRAPHICS_OFFSET_VULKAN_ITEM_MEMORY, GRAPHICS_OFFSET_VULKAN_LIB, GRAPHICS_OFFSET_VULKAN_MAPPED,
    GRAPHICS_OFFSET_VULKAN_PHYSICAL, GRAPHICS_OFFSET_VULKAN_PIPELINE,
    GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT, GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES,
    GRAPHICS_OFFSET_VULKAN_PRESENT_PASS, GRAPHICS_OFFSET_VULKAN_QUEUE,
    GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY, GRAPHICS_OFFSET_VULKAN_READY,
    GRAPHICS_OFFSET_VULKAN_READ_BUFFER, GRAPHICS_OFFSET_VULKAN_READ_MEMORY,
    GRAPHICS_OFFSET_VULKAN_RENDER_PASS, GRAPHICS_OFFSET_VULKAN_SET_LAYOUT,
    GRAPHICS_OFFSET_VULKAN_TEX_HEIGHT, GRAPHICS_OFFSET_VULKAN_TEX_WIDTH, GRAPHICS_STATE_SYMBOL,
    HEADER_AUX0, HEADER_AUX1, HEADER_BLEND, HEADER_BOUNDS, HEADER_CAP, HEADER_FILL_R,
    HEADER_GRADIENT_COUNT, HEADER_GRADIENT_FROM_X, HEADER_GRADIENT_KIND, HEADER_HAS_TRANSFORM,
    HEADER_KIND, HEADER_PICTURE_SHADOW_HI, HEADER_PICTURE_SHADOW_LO, HEADER_POLYLINE_FIRST,
    HEADER_POLYLINE_LAST, HEADER_RADIUS, HEADER_SLOTS, HEADER_STROKE_HALF, HEADER_STROKE_R,
    HEADER_TRANSFORM_IA, HEADER_TRANSFORM_IB, HEADER_TRANSFORM_IC, HEADER_TRANSFORM_ID,
    HEADER_TRANSFORM_ITX, HEADER_TRANSFORM_ITY, ITEM_ARC_CAP, ITEM_ARC_EDGE_BASE,
    ITEM_ARC_GLYPH_HEIGHT, ITEM_BAND_COUNT, ITEM_BAND_HEIGHT, ITEM_BAND_NORM, ITEM_BAND_START,
    ITEM_BAND_TOP, ITEM_BLOCK_SIZE, ITEM_CLIP_BAND_NORM, ITEM_ELLIPSE_GRADIENT_BASE,
    ITEM_ELLIPSE_GRADIENT_COUNT, ITEM_FLOAT_FIELDS, ITEM_OFFSET_ARC, ITEM_OFFSET_ARC_CAPS,
    ITEM_OFFSET_ELLIPSE, ITEM_OFFSET_FILL, ITEM_OFFSET_GRADIENT, ITEM_OFFSET_MISC,
    ITEM_OFFSET_QUAD, ITEM_OFFSET_SHAPE, ITEM_OFFSET_STROKE, ITEM_OFFSET_SURFACE,
    ITEM_OFFSET_TRANSFORM, ITEM_SURFACE_BLEND, ITEM_SURFACE_GRADIENT_KIND, PICTURE_RECORD_BYTES,
    PICTURE_SHADOW_SPLIT_BITS, TILE_SOURCE_H, TILE_SOURCE_W, TILE_SOURCE_X, TILE_SOURCE_Y,
    VULKAN_BAND_BASE_WORDS, VULKAN_BUFFER_BYTES, VULKAN_GLYPH_BASE_WORDS,
    VULKAN_GRADIENT_BASE_WORDS, VULKAN_ITEM_BUFFER_BYTES, VULKAN_MAX_FRAME_BAND_WORDS,
    VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES, VULKAN_MAX_FRAME_GRADIENT_STOPS,
    VULKAN_MAX_FRAME_ITEMS, VULKAN_PICTURE_INDEX_OFFSET, VULKAN_PICTURE_INDEX_SLOTS,
    VULKAN_PICTURE_RECORDS, VULKAN_PICTURE_RECORDS_OFFSET, VULKAN_SPEC_CONSTANTS,
};
use crate::codegen::runtime::canvas::{vulkan_handoff, GRAPHICS_OFFSET_ATTR};
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_PASS_CONTINUES, CANVAS_DRAW_PASS_MORE, CANVAS_DRAW_PASS_WORD,
    GRAPHICS_OFFSET_VULKAN_LOAD_PASS, GRAPHICS_OFFSET_VULKAN_PRESENT_LOAD_PASS,
};
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_REPEAT_WORD, NODE_BYTES, VULKAN_NODE_BASE_WORDS,
};
use crate::codegen::runtime::canvas::{
    HEADER_CLIP_EDGE_COUNT, HEADER_CLIP_MODE, HEADER_CLIP_RULE, HEADER_DASH_COUNT,
    HEADER_DASH_PHASE, HEADER_GRADIENT_APPLY, HEADER_GRADIENT_FOCAL_X, HEADER_GRADIENT_HAS_INV,
    HEADER_GRADIENT_INV, HEADER_GRADIENT_SPREAD, HEADER_JOIN_COUNT, HEADER_JOIN_REACH,
    HEADER_PICTURE_TRANSPARENCY, ITEM_CLIP_PATH_BASE, ITEM_CLIP_PATH_COUNT, ITEM_CLIP_PATH_MODE,
    ITEM_CLIP_PATH_PICTURE_T, ITEM_OFFSET_CLIP_BAND, ITEM_OFFSET_CLIP_PATH,
    ITEM_OFFSET_GRADIENT_FOCAL, ITEM_OFFSET_GRADIENT_INV, ITEM_OFFSET_STROKE_STYLE,
    ITEM_STROKE_DASH_BASE, ITEM_STROKE_DASH_COUNT, ITEM_STROKE_JOIN_BASE, ITEM_STROKE_JOIN_COUNT,
    VULKAN_DASH_BASE_WORDS, VULKAN_MAX_FRAME_DASH_WORDS,
};
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;

mod present;
pub(crate) use present::{CANVAS_CHILD_SYMBOL, WM_APP_CANVAS_SHOW};

mod bands;
mod build;
mod calls;
mod descriptors;
mod draw_scene;
mod edges;
mod item_block;
mod memory;
mod passes;
mod picture;
mod pipeline;
mod publish;
mod stroke_style;
mod target;

use bands::*;
pub(crate) use build::{emit_vulkan_build, emit_vulkan_ready};
use calls::*;
use descriptors::*;
pub(crate) use draw_scene::emit_vulkan_draw_scene;
use edges::*;
use item_block::*;
use memory::*;
use passes::*;
use picture::*;
use pipeline::*;
use publish::*;
use stroke_style::*;
use target::*;

/// The Vulkan loader's soname on Linux.
pub(crate) const VULKAN_SONAME: &str = "libvulkan.so.1";
/// The Vulkan loader's file name on Windows. Loaded by **bare name** through the
/// default DLL search, which finds the loader the ICD installer put in `System32` —
/// the same "must still run on a machine with no Vulkan" rule as the soname above,
/// answered by `LoadLibraryExA` returning NULL rather than by a link-time dependency.
pub(crate) const VULKAN_DLL: &str = "vulkan-1.dll";
/// `RTLD_NOW`; `RTLD_LOCAL` is 0.
const RTLD_NOW: &str = "2";

/// The loader library this platform opens.
pub(crate) fn vulkan_library_name(platform: &dyn CodegenPlatform) -> &'static str {
    match platform.family() {
        PlatformFamily::Windows => VULKAN_DLL,
        _ => VULKAN_SONAME,
    }
}

/// Does this platform have a Vulkan backend at all?
///
/// plan-98-F Phases 1-2 shipped Linux; Phase 3 adds Windows. macOS is deliberately
/// absent and stays that way — it has the Metal backend (plan-98-E), and MoltenVK is
/// a dependency plan-98-A's bar rules out.
pub(crate) fn has_vulkan_backend(platform: &dyn CodegenPlatform) -> bool {
    matches!(
        platform.family(),
        PlatformFamily::Linux | PlatformFamily::Windows
    )
}

/// The one entry point resolved by `dlsym`. Everything else comes from it.
const SYM_GET_INSTANCE_PROC_ADDR: &str = "vkGetInstanceProcAddr";
/// Every entry point the backend resolves, each by name from the loader library.
/// A name with no data object here fails the *build* — `emit_data_address` has
/// nothing to point at — rather than silently resolving to null at run time.
const VK_PROBE_ENTRY_POINTS: &[&str] = &[
    "vkCreateInstance",
    "vkEnumeratePhysicalDevices",
    "vkDestroyInstance",
    "vkGetPhysicalDeviceQueueFamilyProperties",
    "vkCreateDevice",
    "vkGetDeviceQueue",
    "vkCreateShaderModule",
    "vkCreatePipelineLayout",
    "vkCreateRenderPass",
    "vkCreateGraphicsPipelines",
    "vkDeviceWaitIdle",
    "vkDestroyFramebuffer",
    "vkDestroyImageView",
    "vkDestroyImage",
    "vkDestroyBuffer",
    "vkFreeMemory",
    "vkCreateImage",
    "vkGetImageMemoryRequirements",
    "vkGetPhysicalDeviceMemoryProperties",
    "vkAllocateMemory",
    "vkBindImageMemory",
    "vkCreateImageView",
    "vkCreateFramebuffer",
    "vkCreateBuffer",
    "vkGetBufferMemoryRequirements",
    "vkBindBufferMemory",
    "vkMapMemory",
    "vkCreateDescriptorSetLayout",
    "vkCreateDescriptorPool",
    "vkAllocateDescriptorSets",
    "vkUpdateDescriptorSets",
    "vkCreateCommandPool",
    "vkAllocateCommandBuffers",
    "vkBeginCommandBuffer",
    "vkCmdBeginRenderPass",
    "vkCmdBindPipeline",
    "vkCmdBindDescriptorSets",
    "vkCmdSetViewport",
    "vkCmdSetScissor",
    "vkCmdPushConstants",
    "vkCmdDraw",
    "vkCmdEndRenderPass",
    "vkCmdCopyImageToBuffer",
    "vkEndCommandBuffer",
    "vkQueueSubmit",
    "vkQueueWaitIdle",
    // bug-688 V6: the swapchain present (`present/`). Resolved only when a
    // frame presents, so a driver without them still renders through readback.
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
];

/// `VK_API_VERSION_1_0` — `VK_MAKE_API_VERSION(0, 1, 0, 0)`, i.e. `1 << 22`.
const VK_API_VERSION_1_0: &str = "4194304";

// ---------------------------------------------------------------------------------
// Vulkan constants and struct layouts.
//
// Every offset below is a hand-transcribed C layout — there is no header to include
// and no compiler to disagree with, so a wrong one does not fail the build, it hands
// garbage to a driver. Each is written beside the member it belongs to, and
// `layout_tables_are_self_consistent` checks the property a transcription slip
// almost always breaks: members inside their struct at their natural alignment, no
// two overlapping, sizes a multiple of the struct's alignment.
// ---------------------------------------------------------------------------------

/// `VkStructureType` values, in the order they are first used.
const ST_APPLICATION_INFO: &str = "0";
const ST_INSTANCE_CREATE_INFO: &str = "1";
const ST_DEVICE_QUEUE_CREATE_INFO: &str = "2";
const ST_DEVICE_CREATE_INFO: &str = "3";

/// `1.0f` as its IEEE-754 bit pattern, for `lineWidth` and the queue priority.
const FLOAT_ONE_BITS: &str = "1065353216";
/// `VK_QUEUE_GRAPHICS_BIT`.
const QUEUE_GRAPHICS_BIT: &str = "1";

// `VkApplicationInfo`, 48 bytes. The 4-byte `sType` is followed by 4 bytes of
// padding before the 8-byte `pNext`, and likewise after each 4-byte member that
// precedes a pointer — the offsets are the result, not a guess.
const APP_INFO_SIZE: usize = 48;
const APP_INFO_STYPE: usize = 0;
const APP_INFO_API_VERSION: usize = 44;

// `VkInstanceCreateInfo`, 64 bytes.
const INSTANCE_INFO_SIZE: usize = 64;
const INSTANCE_INFO_STYPE: usize = 0;
const INSTANCE_INFO_APP_INFO: usize = 24;

/// The compiled shaders.
///
/// Vulkan takes SPIR-V, not source — unlike Metal, which compiles its MSL string at
/// run time. So the blobs are checked in beside the GLSL they came from and embedded
/// here; `scripts/regen-spirv.sh` reproduces them.
const SPIRV_VERTEX: &[u8] = include_bytes!("../shaders/mfb_canvas.vert.spv");
const SPIRV_FRAGMENT: &[u8] = include_bytes!("../shaders/mfb_canvas.frag.spv");

/// SPIR-V's magic word, read as a little-endian `u32`. Only the well-formedness
/// test reads it — the emitter hands the blob to the driver without inspecting it,
/// which is exactly why that test exists.
#[cfg(test)]
const SPIRV_MAGIC: u32 = 0x0723_0203;

/// Both shaders' entry point. GLSL compiled by glslang always names it `main`.
const SHADER_ENTRY_NAME: &str = "main";

/// `VkStructureType` values used by the pipeline.
const ST_SHADER_MODULE_CREATE_INFO: &str = "16";
const ST_PIPELINE_SHADER_STAGE_CREATE_INFO: &str = "18";
const ST_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO: &str = "19";
const ST_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO: &str = "20";
const ST_PIPELINE_VIEWPORT_STATE_CREATE_INFO: &str = "22";
const ST_PIPELINE_RASTERIZATION_STATE_CREATE_INFO: &str = "23";
const ST_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO: &str = "24";
const ST_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO: &str = "26";
const ST_PIPELINE_DYNAMIC_STATE_CREATE_INFO: &str = "27";
const ST_GRAPHICS_PIPELINE_CREATE_INFO: &str = "28";
const ST_PIPELINE_LAYOUT_CREATE_INFO: &str = "30";
const ST_RENDER_PASS_CREATE_INFO: &str = "38";

/// `VK_FORMAT_B8G8R8A8_SRGB` — the sRGB-encoding format, so the GPU does the encode
/// exactly where the software oracle's `__COLOR_SRGB` table does, and the same
/// format the Metal pipeline targets.
const FORMAT_B8G8R8A8_SRGB: &str = "50";
/// `VK_SAMPLE_COUNT_1_BIT`.
const SAMPLE_COUNT_1: &str = "1";
/// `VK_ATTACHMENT_LOAD_OP_CLEAR` / `VK_ATTACHMENT_STORE_OP_STORE`.
const ATTACHMENT_LOAD_OP_CLEAR: &str = "1";
/// `VK_ATTACHMENT_LOAD_OP_LOAD`: keep what the attachment holds (bug-765, a frame's
/// passes after its first).
const ATTACHMENT_LOAD_OP_LOAD: &str = "0";
const ATTACHMENT_STORE_OP_STORE: &str = "0";
/// `VK_IMAGE_LAYOUT_UNDEFINED` / `..._COLOR_ATTACHMENT_OPTIMAL` /
/// `..._TRANSFER_SRC_OPTIMAL` — the render pass ends in TRANSFER_SRC so the frame
/// can be copied straight out to the readback buffer.
const IMAGE_LAYOUT_UNDEFINED: &str = "0";
const IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL: &str = "2";
const IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL: &str = "6";
/// `VK_PIPELINE_BIND_POINT_GRAPHICS`.
const PIPELINE_BIND_POINT_GRAPHICS: &str = "0";
/// `VK_PRIMITIVE_TOPOLOGY_TRIANGLE_STRIP` — four vertices, two triangles, and (as on
/// Metal) no vertex buffer, so there is nothing to keep in sync with a CPU layout.
///
/// 4, not 5: the enum runs POINT_LIST, LINE_LIST, LINE_STRIP, TRIANGLE_LIST,
/// TRIANGLE_STRIP, TRIANGLE_FAN, so 5 is the *fan*. A fan over strip-ordered
/// vertices is not an error — it draws two real triangles that happen not to be the
/// quad, which came out as a shape missing its lower-right corner. The Metal path hit
/// the same class of bug from the same off-by-one in a different enum.
const TOPOLOGY_TRIANGLE_STRIP: &str = "4";
/// `VK_POLYGON_MODE_FILL`, `VK_CULL_MODE_NONE`, `VK_FRONT_FACE_COUNTER_CLOCKWISE`.
const POLYGON_MODE_FILL: &str = "0";
const CULL_MODE_NONE: &str = "0";
const FRONT_FACE_CCW: &str = "0";
/// `VK_BLEND_FACTOR_ONE` / `VK_BLEND_FACTOR_ONE_MINUS_SRC_ALPHA` — the premultiplied
/// `over` the software oracle implements by hand, and the pair the Metal pipeline
/// uses.
const BLEND_FACTOR_ONE: &str = "1";
const BLEND_FACTOR_ONE_MINUS_SRC_ALPHA: &str = "7";
/// `VK_BLEND_FACTOR_ONE_MINUS_SRC_COLOR` (3) and `VK_BLEND_FACTOR_DST_COLOR` (4) —
/// the two extra factors plan-116-B's `Screen` and `Multiply` pipelines need.
///
/// From `VkBlendFactor`: 0 ZERO, 1 ONE, 2 SRC_COLOR, 3 ONE_MINUS_SRC_COLOR,
/// 4 DST_COLOR, 5 ONE_MINUS_DST_COLOR, 6 SRC_ALPHA, 7 ONE_MINUS_SRC_ALPHA. Worth
/// spelling out: `DST_COLOR` is 4, and mistaking it for one of the alpha factors
/// would produce a picture that still looked like a blend.
const BLEND_FACTOR_ONE_MINUS_SRC_COLOR: &str = "3";
const BLEND_FACTOR_DST_COLOR: &str = "4";
/// `VK_BLEND_OP_ADD`, and the full `VK_COLOR_COMPONENT_*_BIT` mask.
const BLEND_OP_ADD: &str = "0";
const COLOR_COMPONENT_RGBA: &str = "15";
/// `VK_SHADER_STAGE_VERTEX_BIT` / `..._FRAGMENT_BIT`, and the pair, for the push
/// constant range both stages read.
const SHADER_STAGE_VERTEX: &str = "1";
const SHADER_STAGE_FRAGMENT: &str = "16";
const SHADER_STAGE_VERTEX_AND_FRAGMENT: &str = "17";

pub(crate) fn vertex_spirv_symbol() -> String {
    "_mfb_canvas_vk_spirv_vert".to_string()
}

pub(crate) fn fragment_spirv_symbol() -> String {
    "_mfb_canvas_vk_spirv_frag".to_string()
}

pub(crate) fn entry_name_symbol() -> String {
    "_mfb_canvas_vk_entry_name".to_string()
}

/// The two SPIR-V blobs, as read-only data.
///
/// `align: 4` is required, not cosmetic: `VkShaderModuleCreateInfo::pCode` is a
/// `const uint32_t*`, and a driver may fault or reject a misaligned pointer.
fn spirv_data_objects() -> Vec<CodeDataObject> {
    [
        (vertex_spirv_symbol(), SPIRV_VERTEX),
        (fragment_spirv_symbol(), SPIRV_FRAGMENT),
    ]
    .into_iter()
    .map(|(symbol, bytes)| CodeDataObject {
        symbol,
        kind: "raw".to_string(),
        layout: "SPIR-V module".to_string(),
        align: 4,
        size: bytes.len(),
        value: bytes.iter().map(|byte| format!("{byte:02x}")).collect(),
    })
    .collect()
}

/// The C strings the loader references: the soname and every symbol name.
pub(crate) fn data_objects(platform: &dyn CodegenPlatform) -> Vec<CodeDataObject> {
    let library = vulkan_library_name(platform);
    let mut objects = vec![CodeDataObject {
        symbol: soname_symbol(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: library.len() + 1,
        value: hex_encode_cstring(library),
    }];
    for name in std::iter::once(&SYM_GET_INSTANCE_PROC_ADDR).chain(VK_PROBE_ENTRY_POINTS) {
        objects.push(CodeDataObject {
            symbol: symbol_name_symbol(name),
            kind: "raw".to_string(),
            layout: "C string (NUL-terminated)".to_string(),
            align: 1,
            size: name.len() + 1,
            value: hex_encode_cstring(name),
        });
    }
    objects.push(CodeDataObject {
        symbol: entry_name_symbol(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: SHADER_ENTRY_NAME.len() + 1,
        value: hex_encode_cstring(SHADER_ENTRY_NAME),
    });
    objects.extend(spirv_data_objects());
    if present::has_present_path(platform) {
        objects.extend(present::wsi_data_objects());
    }
    objects
}

pub(crate) fn soname_symbol() -> String {
    "_mfb_canvas_vk_soname".to_string()
}

pub(crate) fn symbol_name_symbol(name: &str) -> String {
    format!("_mfb_canvas_vk_sym_{name}")
}

// --- the descriptor set that binds the polygon edge buffer -----------------------

/// `VkDescriptorSetLayoutBinding`, 24 bytes.
const BINDING_SIZE: usize = 24;
const BINDING_INDEX: usize = 0;
const BINDING_TYPE: usize = 4;
const BINDING_COUNT: usize = 8;
const BINDING_STAGES: usize = 12;

/// `VkDescriptorSetLayoutCreateInfo`, 32 bytes.
const SET_LAYOUT_INFO_SIZE: usize = 32;
const SET_LAYOUT_BINDING_COUNT: usize = 20;
const SET_LAYOUT_BINDINGS: usize = 24;

/// `VkDescriptorPoolSize`, 8 bytes.
const POOL_SIZE_SIZE: usize = 8;
const POOL_SIZE_TYPE: usize = 0;
const POOL_SIZE_COUNT: usize = 4;

/// `VkDescriptorPoolCreateInfo`, 40 bytes.
const DESC_POOL_INFO_SIZE: usize = 40;
const DESC_POOL_MAX_SETS: usize = 20;
const DESC_POOL_SIZE_COUNT: usize = 24;
const DESC_POOL_SIZES: usize = 32;

/// `VkDescriptorSetAllocateInfo`, 40 bytes.
const DESC_ALLOC_INFO_SIZE: usize = 40;
const DESC_ALLOC_POOL: usize = 16;
const DESC_ALLOC_SET_COUNT: usize = 24;
const DESC_ALLOC_LAYOUTS: usize = 32;

/// `VkDescriptorBufferInfo`, 24 bytes.
const DESC_BUFFER_INFO_SIZE: usize = 24;
const DESC_BUFFER_INFO_BUFFER: usize = 0;
const DESC_BUFFER_INFO_RANGE: usize = 16;

/// `VkWriteDescriptorSet`, 64 bytes.
const WRITE_SET_SIZE: usize = 64;
const WRITE_SET_DST: usize = 16;
const WRITE_SET_BINDING: usize = 24;
const WRITE_SET_COUNT: usize = 32;
const WRITE_SET_TYPE: usize = 36;
const WRITE_SET_BUFFER_INFO: usize = 48;

const ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO: &str = "32";
const ST_DESCRIPTOR_POOL_CREATE_INFO: &str = "33";
const ST_DESCRIPTOR_SET_ALLOCATE_INFO: &str = "34";
const ST_WRITE_DESCRIPTOR_SET: &str = "35";

/// `VK_DESCRIPTOR_TYPE_STORAGE_BUFFER`.
const DESCRIPTOR_TYPE_STORAGE_BUFFER: &str = "7";
/// `VK_BUFFER_USAGE_STORAGE_BUFFER_BIT`.
const BUFFER_USAGE_STORAGE: &str = "32";
/// `VkDeviceQueueCreateInfo`, 40 bytes.
const QUEUE_INFO_SIZE: usize = 40;
const QUEUE_INFO_STYPE: usize = 0;
const QUEUE_INFO_FAMILY: usize = 20;
const QUEUE_INFO_COUNT: usize = 24;
const QUEUE_INFO_PRIORITIES: usize = 32;

/// `VkDeviceCreateInfo`, 72 bytes.
const DEVICE_INFO_SIZE: usize = 72;
const DEVICE_INFO_STYPE: usize = 0;
const DEVICE_INFO_QUEUE_COUNT: usize = 20;
const DEVICE_INFO_QUEUE_INFOS: usize = 24;

/// `VkQueueFamilyProperties`, 24 bytes; only `queueFlags` is read.
const QUEUE_FAMILY_PROPERTIES_SIZE: usize = 24;
const QUEUE_FAMILY_FLAGS: usize = 0;

/// How many physical devices and queue families the probe will look at.
///
/// Both are fixed rather than allocated: the arrays live on the frame, and a machine
/// with more than eight of either still works — Vulkan fills as many as the count
/// says and reports `VK_INCOMPLETE`, which is not an error for "find me a graphics
/// queue". Growing the frame to hold every possible family would trade a real
/// allocation for a case that does not exist.
const MAX_PHYSICAL_DEVICES: usize = 8;
const MAX_QUEUE_FAMILIES: usize = 8;

// --- pipeline struct layouts -----------------------------------------------------

/// `VkShaderModuleCreateInfo`, 40 bytes.
const SHADER_MODULE_INFO_SIZE: usize = 40;
const SHADER_MODULE_STYPE: usize = 0;
const SHADER_MODULE_CODE_SIZE: usize = 24;
const SHADER_MODULE_CODE: usize = 32;

/// `VkPipelineLayoutCreateInfo`, 48 bytes.
const LAYOUT_INFO_SIZE: usize = 48;
const LAYOUT_INFO_STYPE: usize = 0;
const LAYOUT_INFO_SET_COUNT: usize = 20;
const LAYOUT_INFO_SETS: usize = 24;
const LAYOUT_INFO_RANGE_COUNT: usize = 32;
/// `pPushConstantRanges`.
///
/// plan-116-A set `rangeCount` to 0 and left this null, and its comment was right at the
/// time: *"a layout that declares bytes no stage consumes is a layout the validation
/// layers flag"*. plan-116-H re-adds one range because both stages now consume it — the
/// vertex stage offsets its quad, the fragment stage subtracts before evaluating the
/// distance field (**H4**). Re-adding the range and the two shader declarations has to
/// happen in ONE step for exactly the reason A deleted it: either half alone is an
/// invalid layout, and invalid in a way that only surfaces on a box running the
/// validation layers.
const LAYOUT_INFO_RANGES: usize = 40;

/// `VkPushConstantRange`, 12 bytes: `stageFlags`, `offset`, `size`.
const PUSH_RANGE_SIZE: usize = 12;
const PUSH_RANGE_STAGES: usize = 0;
const PUSH_RANGE_OFFSET: usize = 4;
const PUSH_RANGE_BYTES: usize = 8;

/// plan-175-B: the per-draw push constant, `uniform Run` in the VERTEX stage only: four
/// 32-bit words `{ base, count, node, repeat }`. The group offset and inverse that were
/// pushed per draw until this letter (`uniform Draw`, 32 bytes, both stages) are the
/// row's NODE now, in the shared buffer's node region (`VULKAN_NODE_BASE_WORDS`), read by
/// both stages by index.
///
/// It is the *only* push constant, so its offset is 0. The pipeline layout's range, the
/// `vkCmdPushConstants` size and the stack slot the values are staged in all read this
/// one constant.
const PUSH_RUN_BYTES: usize = 16;

/// `VkAttachmentDescription`, 36 bytes.
const ATTACHMENT_SIZE: usize = 36;
const ATTACHMENT_FORMAT: usize = 4;
const ATTACHMENT_SAMPLES: usize = 8;
const ATTACHMENT_LOAD_OP: usize = 12;
const ATTACHMENT_STORE_OP: usize = 16;
const ATTACHMENT_INITIAL_LAYOUT: usize = 28;
const ATTACHMENT_FINAL_LAYOUT: usize = 32;

/// `VkAttachmentReference`, 8 bytes.
const ATTACHMENT_REF_SIZE: usize = 8;
const ATTACHMENT_REF_INDEX: usize = 0;
const ATTACHMENT_REF_LAYOUT: usize = 4;

/// `VkSubpassDescription`, 72 bytes.
const SUBPASS_SIZE: usize = 72;
const SUBPASS_BIND_POINT: usize = 4;
const SUBPASS_COLOR_COUNT: usize = 24;
const SUBPASS_COLOR_ATTACHMENTS: usize = 32;

/// `VkRenderPassCreateInfo`, 64 bytes.
const RENDER_PASS_INFO_SIZE: usize = 64;
const RENDER_PASS_STYPE: usize = 0;
const RENDER_PASS_ATTACHMENT_COUNT: usize = 20;
const RENDER_PASS_ATTACHMENTS: usize = 24;
const RENDER_PASS_SUBPASS_COUNT: usize = 32;
const RENDER_PASS_SUBPASSES: usize = 40;

/// `VkPipelineShaderStageCreateInfo`, 48 bytes.
const STAGE_INFO_SIZE: usize = 48;
const STAGE_INFO_STYPE: usize = 0;
const STAGE_INFO_STAGE: usize = 20;
const STAGE_INFO_MODULE: usize = 24;
const STAGE_INFO_NAME: usize = 32;
/// `pSpecializationInfo` — the fragment stage's region bases (bug-688).
const STAGE_INFO_SPECIALIZATION: usize = 40;

/// `VkSpecializationInfo`, 32 bytes: `mapEntryCount`, then `pMapEntries`, `dataSize`
/// and `pData`, each 8-aligned.
const SPEC_INFO_SIZE: usize = 32;
const SPEC_INFO_COUNT: usize = 0;
const SPEC_INFO_ENTRIES: usize = 8;
const SPEC_INFO_DATA_SIZE: usize = 16;
const SPEC_INFO_DATA: usize = 24;
/// `VkSpecializationMapEntry`, 16 bytes: `constantID`, `offset`, then `size` as a
/// `size_t`.
const SPEC_ENTRY_SIZE: usize = 16;
const SPEC_ENTRY_ID: usize = 0;
const SPEC_ENTRY_OFFSET: usize = 4;
const SPEC_ENTRY_BYTES: usize = 8;

/// `VkPipelineVertexInputStateCreateInfo`, 48 bytes — all zero but the `sType`,
/// because the vertex shader synthesizes its four corners from `gl_VertexIndex`.
const VERTEX_INPUT_INFO_SIZE: usize = 48;

/// `VkPipelineInputAssemblyStateCreateInfo`, 32 bytes.
const INPUT_ASSEMBLY_INFO_SIZE: usize = 32;
const INPUT_ASSEMBLY_TOPOLOGY: usize = 20;

/// `VkPipelineViewportStateCreateInfo`, 48 bytes. The viewport and scissor are
/// **dynamic**, so the pointers stay null and only the counts matter — which is what
/// lets a resize reuse the pipeline instead of rebuilding it.
const VIEWPORT_INFO_SIZE: usize = 48;
const VIEWPORT_INFO_VIEWPORT_COUNT: usize = 20;
const VIEWPORT_INFO_SCISSOR_COUNT: usize = 32;

/// `VkPipelineRasterizationStateCreateInfo`, 64 bytes.
const RASTER_INFO_SIZE: usize = 64;
const RASTER_POLYGON_MODE: usize = 28;
const RASTER_CULL_MODE: usize = 32;
const RASTER_FRONT_FACE: usize = 36;
const RASTER_LINE_WIDTH: usize = 56;

/// `VkPipelineMultisampleStateCreateInfo`, 48 bytes.
const MULTISAMPLE_INFO_SIZE: usize = 48;
const MULTISAMPLE_SAMPLES: usize = 20;

/// `VkPipelineColorBlendAttachmentState`, 32 bytes — the premultiplied `over` the
/// software oracle implements by hand, and the same pair the Metal pipeline uses.
const BLEND_ATTACHMENT_SIZE: usize = 32;
const BLEND_ENABLE: usize = 0;
const BLEND_SRC_COLOR: usize = 4;
const BLEND_DST_COLOR: usize = 8;
const BLEND_COLOR_OP: usize = 12;
const BLEND_SRC_ALPHA: usize = 16;
const BLEND_DST_ALPHA: usize = 20;
const BLEND_ALPHA_OP: usize = 24;
const BLEND_WRITE_MASK: usize = 28;

/// `VkPipelineColorBlendStateCreateInfo`, 56 bytes.
const BLEND_INFO_SIZE: usize = 56;
const BLEND_INFO_ATTACHMENT_COUNT: usize = 28;
const BLEND_INFO_ATTACHMENTS: usize = 32;

/// `VkPipelineDynamicStateCreateInfo`, 32 bytes.
const DYNAMIC_INFO_SIZE: usize = 32;
const DYNAMIC_INFO_COUNT: usize = 20;
const DYNAMIC_INFO_STATES: usize = 24;

/// `VkGraphicsPipelineCreateInfo`, 144 bytes.
const PIPELINE_INFO_SIZE: usize = 144;
const PIPELINE_STAGE_COUNT: usize = 20;
const PIPELINE_STAGES: usize = 24;
const PIPELINE_VERTEX_INPUT: usize = 32;
const PIPELINE_INPUT_ASSEMBLY: usize = 40;
const PIPELINE_VIEWPORT: usize = 56;
const PIPELINE_RASTER: usize = 64;
const PIPELINE_MULTISAMPLE: usize = 72;
const PIPELINE_COLOR_BLEND: usize = 88;
const PIPELINE_DYNAMIC: usize = 96;
const PIPELINE_LAYOUT: usize = 104;
const PIPELINE_RENDER_PASS: usize = 112;

// --- render-target struct layouts ------------------------------------------------

/// `VkImageCreateInfo`, 88 bytes.
const IMAGE_INFO_SIZE: usize = 88;
const IMAGE_INFO_TYPE: usize = 20;
const IMAGE_INFO_FORMAT: usize = 24;
const IMAGE_INFO_WIDTH: usize = 28;
const IMAGE_INFO_HEIGHT: usize = 32;
const IMAGE_INFO_DEPTH: usize = 36;
const IMAGE_INFO_MIP_LEVELS: usize = 40;
const IMAGE_INFO_ARRAY_LAYERS: usize = 44;
const IMAGE_INFO_SAMPLES: usize = 48;
const IMAGE_INFO_TILING: usize = 52;
const IMAGE_INFO_USAGE: usize = 56;
const IMAGE_INFO_INITIAL_LAYOUT: usize = 80;

/// `VkMemoryRequirements`, 24 bytes.
const MEMORY_REQS_SIZE: usize = 24;
const MEMORY_REQS_BYTES: usize = 0;
const MEMORY_REQS_TYPE_BITS: usize = 16;

/// `VkMemoryAllocateInfo`, 32 bytes.
const ALLOC_INFO_SIZE: usize = 32;
const ALLOC_INFO_BYTES: usize = 16;
const ALLOC_INFO_TYPE_INDEX: usize = 24;

/// `VkPhysicalDeviceMemoryProperties`, 520 bytes: a count, then 32
/// `VkMemoryType`s of `{ propertyFlags u32, heapIndex u32 }`.
const MEMORY_PROPERTIES_SIZE: usize = 520;
const MEMORY_PROPERTIES_TYPE_COUNT: usize = 0;
const MEMORY_PROPERTIES_TYPES: usize = 4;
const MEMORY_TYPE_STRIDE: usize = 8;

/// `VkImageViewCreateInfo`, 80 bytes.
const VIEW_INFO_SIZE: usize = 80;
const VIEW_INFO_IMAGE: usize = 24;
const VIEW_INFO_TYPE: usize = 32;
const VIEW_INFO_FORMAT: usize = 36;
const VIEW_INFO_ASPECT: usize = 56;
const VIEW_INFO_LEVEL_COUNT: usize = 64;
const VIEW_INFO_LAYER_COUNT: usize = 72;

/// `VkFramebufferCreateInfo`, 64 bytes.
const FRAMEBUFFER_INFO_SIZE: usize = 64;
const FRAMEBUFFER_RENDER_PASS: usize = 24;
const FRAMEBUFFER_ATTACHMENT_COUNT: usize = 32;
const FRAMEBUFFER_ATTACHMENTS: usize = 40;
const FRAMEBUFFER_WIDTH: usize = 48;
const FRAMEBUFFER_HEIGHT: usize = 52;
const FRAMEBUFFER_LAYERS: usize = 56;

/// `VkBufferCreateInfo`, 56 bytes.
const BUFFER_INFO_SIZE: usize = 56;
const BUFFER_INFO_BYTES: usize = 24;
const BUFFER_INFO_USAGE: usize = 32;

/// `VkCommandPoolCreateInfo`, 24 bytes.
const POOL_INFO_SIZE: usize = 24;
const POOL_INFO_FLAGS: usize = 16;
const POOL_INFO_FAMILY: usize = 20;

/// `VkCommandBufferAllocateInfo`, 32 bytes.
const CMD_ALLOC_INFO_SIZE: usize = 32;
const CMD_ALLOC_POOL: usize = 16;
const CMD_ALLOC_LEVEL: usize = 24;
const CMD_ALLOC_COUNT: usize = 28;

/// `VK_STRUCTURE_TYPE_*` for the target.
const ST_MEMORY_ALLOCATE_INFO: &str = "5";
const ST_BUFFER_CREATE_INFO: &str = "12";
const ST_IMAGE_CREATE_INFO: &str = "14";
const ST_IMAGE_VIEW_CREATE_INFO: &str = "15";
const ST_FRAMEBUFFER_CREATE_INFO: &str = "37";
const ST_COMMAND_POOL_CREATE_INFO: &str = "39";
const ST_COMMAND_BUFFER_ALLOCATE_INFO: &str = "40";

/// `VK_IMAGE_TYPE_2D` / `VK_IMAGE_VIEW_TYPE_2D`.
const IMAGE_TYPE_2D: &str = "1";
const IMAGE_VIEW_TYPE_2D: &str = "1";
/// `VK_IMAGE_TILING_OPTIMAL`.
const IMAGE_TILING_OPTIMAL: &str = "0";
/// `VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT | VK_IMAGE_USAGE_TRANSFER_SRC_BIT`.
const IMAGE_USAGE_COLOR_AND_SRC: &str = "17";
/// `VK_IMAGE_ASPECT_COLOR_BIT`.
const IMAGE_ASPECT_COLOR: &str = "1";
/// `VK_BUFFER_USAGE_TRANSFER_DST_BIT`.
const BUFFER_USAGE_TRANSFER_DST: &str = "2";
/// `VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT`, and
/// `HOST_VISIBLE | HOST_COHERENT` for the readback buffer — coherent so the copy
/// needs no explicit invalidate before the CPU reads it.
const MEMORY_DEVICE_LOCAL: &str = "1";
const MEMORY_HOST_VISIBLE_COHERENT: &str = "6";
/// `HOST_VISIBLE | HOST_COHERENT | HOST_CACHED` — the readback buffer's first
/// choice (bug-688 V5). The CPU reads every pixel back each frame; on an integrated
/// GPU the first coherent type is write-combined, which makes those reads uncached:
/// ~200 ms a frame for a 900x640 surface on an Intel UHD 630 (box 206), against a
/// cached type the same driver also offers. Coherent, so no invalidate is needed.
const MEMORY_HOST_READBACK_CACHED: &str = "14";
/// The memory-type candidates for every buffer the CPU writes or reads through a
/// persistent map — the readback buffer, the shared edge/band/gradient/picture
/// buffer and the item buffer — in preference order: cached if the device has it,
/// else the plain coherent type every Vulkan device must offer. The shared buffer is
/// also READ by the CPU (the band-index builder reads back the edges it just wrote),
/// which on write-combined memory cost 100 x 300-point polygons 62 -> 27 ms a frame
/// and 10 x 4,000-point polygons 81 -> 35 ms on box 206.
const READBACK_MEMORY_CANDIDATES: [&str; 2] =
    [MEMORY_HOST_READBACK_CACHED, MEMORY_HOST_VISIBLE_COHERENT];
/// `VK_COMMAND_POOL_CREATE_RESET_COMMAND_BUFFER_BIT` — the one buffer is re-recorded
/// every frame.
const POOL_RESET_COMMAND_BUFFER: &str = "2";
/// `VK_COMMAND_BUFFER_LEVEL_PRIMARY`.
const COMMAND_BUFFER_LEVEL_PRIMARY: &str = "0";

/// `VkCommandBufferBeginInfo`, 32 bytes.
const CMD_BEGIN_INFO_SIZE: usize = 32;
const CMD_BEGIN_FLAGS: usize = 16;

/// `VkRenderPassBeginInfo`, 64 bytes; `renderArea` is a `VkRect2D` inline at 32.
const PASS_BEGIN_INFO_SIZE: usize = 64;
const PASS_BEGIN_RENDER_PASS: usize = 16;
const PASS_BEGIN_FRAMEBUFFER: usize = 24;
const PASS_BEGIN_AREA_WIDTH: usize = 40;
const PASS_BEGIN_AREA_HEIGHT: usize = 44;
const PASS_BEGIN_CLEAR_COUNT: usize = 48;
const PASS_BEGIN_CLEARS: usize = 56;

/// `VkClearValue`, 16 bytes — four floats. Opaque black, matching
/// `canvas::newSurface`, so both backends start from the same pixels.
const CLEAR_VALUE_SIZE: usize = 16;
const CLEAR_ALPHA: usize = 12;

/// `VkViewport`, 24 bytes of floats, and `VkRect2D`, 16 bytes of ints.
const VIEWPORT_SIZE: usize = 24;
const VIEWPORT_WIDTH: usize = 8;
const VIEWPORT_HEIGHT: usize = 12;
const VIEWPORT_MAX_DEPTH: usize = 20;
const RECT_SIZE: usize = 16;
const RECT_WIDTH: usize = 8;
const RECT_HEIGHT: usize = 12;

/// `VkBufferImageCopy`, 56 bytes.
const COPY_SIZE: usize = 56;
const COPY_ASPECT: usize = 16;
const COPY_LAYER_COUNT: usize = 28;
const COPY_EXTENT_WIDTH: usize = 44;
const COPY_EXTENT_HEIGHT: usize = 48;
const COPY_EXTENT_DEPTH: usize = 52;

/// `VkSubmitInfo`, 72 bytes.
const SUBMIT_INFO_SIZE: usize = 72;
const SUBMIT_COMMAND_COUNT: usize = 40;
const SUBMIT_COMMANDS: usize = 48;

const ST_SUBMIT_INFO: &str = "4";
const ST_COMMAND_BUFFER_BEGIN_INFO: &str = "42";
const ST_RENDER_PASS_BEGIN_INFO: &str = "43";

/// `VK_COMMAND_BUFFER_USAGE_ONE_TIME_SUBMIT_BIT`.
const COMMAND_BUFFER_ONE_TIME_SUBMIT: &str = "1";
/// `VK_SUBPASS_CONTENTS_INLINE`.
const SUBPASS_CONTENTS_INLINE: &str = "0";
/// `1.0f` as its IEEE-754 bit pattern, for the viewport's `maxDepth` and the clear
/// colour's alpha.
const FLOAT_ONE_BITS_F32: &str = "1065353216";

#[cfg(test)]
mod tests;
