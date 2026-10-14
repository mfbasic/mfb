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
    emit_item_arc_flags, emit_item_polygon_rule, emit_item_residual_field, emit_item_stroke_sign,
    emit_square_cap_reach, push_symbol_address, BAND_LIST_PER_EDGE, BAND_MAX, BAND_MIN_EDGES,
    BLEND_MODE_COUNT, CANVAS_DRAW_ENTRY_COUNT_SHIFT, CANVAS_DRAW_ENTRY_MODE,
    CANVAS_DRAW_ENTRY_SHIFT, CURVE_PIECE_SLOTS, EDGE_RESIDUAL_SCALE, EDGE_SLOTS, FIXED_POINT_SCALE,
    GEO_KIND_CURVEPOLYGON, GEO_KIND_PICTURE, GEO_KIND_POLYCURVE, GEO_KIND_POLYGON,
    GEO_KIND_POLYLINE, GEO_KIND_TEXT, GEO_KIND_TILE, GLYPH_META_H, GLYPH_META_SLOTS,
    GLYPH_META_START, GLYPH_META_W, GLYPH_META_X0, GLYPH_META_Y0, GLYPH_RUN_SLOTS,
    GRADIENT_STOP_WORDS, GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER,
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
    ITEM_ARC_GLYPH_HEIGHT, ITEM_BAND_COUNT, ITEM_BAND_HEIGHT, ITEM_BAND_START, ITEM_BAND_TOP,
    ITEM_BLOCK_SIZE, ITEM_ELLIPSE_GRADIENT_BASE, ITEM_ELLIPSE_GRADIENT_COUNT, ITEM_OFFSET_ARC,
    ITEM_OFFSET_ARC_CAPS, ITEM_OFFSET_ELLIPSE, ITEM_OFFSET_FILL, ITEM_OFFSET_GRADIENT,
    ITEM_OFFSET_MISC, ITEM_OFFSET_MISC_LO, ITEM_OFFSET_QUAD, ITEM_OFFSET_SHAPE, ITEM_OFFSET_STROKE,
    ITEM_OFFSET_SURFACE, ITEM_OFFSET_TRANSFORM, ITEM_RESIDUAL_FIELDS, ITEM_SURFACE_BLEND,
    ITEM_SURFACE_GRADIENT_KIND, PICTURE_RECORD_BYTES, PICTURE_SHADOW_SPLIT_BITS, TILE_SOURCE_H,
    TILE_SOURCE_W, TILE_SOURCE_X, TILE_SOURCE_Y, VULKAN_BAND_BASE_WORDS, VULKAN_BUFFER_BYTES,
    VULKAN_EDGE_RESIDUAL_BASE_WORDS, VULKAN_GLYPH_BASE_WORDS, VULKAN_GRADIENT_BASE_WORDS,
    VULKAN_ITEM_BUFFER_BYTES, VULKAN_MAX_FRAME_BAND_WORDS, VULKAN_MAX_FRAME_EDGES,
    VULKAN_MAX_FRAME_GLYPH_SAMPLES, VULKAN_MAX_FRAME_GRADIENT_STOPS, VULKAN_MAX_FRAME_ITEMS,
    VULKAN_PICTURE_INDEX_OFFSET, VULKAN_PICTURE_INDEX_SLOTS, VULKAN_PICTURE_RECORDS,
    VULKAN_PICTURE_RECORDS_OFFSET, VULKAN_SPEC_CONSTANTS,
};
use crate::codegen::runtime::canvas::{vulkan_handoff, GRAPHICS_OFFSET_ATTR};
use crate::codegen::runtime::canvas::{
    CANVAS_DRAW_PASS_CONTINUES, CANVAS_DRAW_PASS_MORE, CANVAS_DRAW_PASS_WORD,
    GRAPHICS_OFFSET_VULKAN_LOAD_PASS, GRAPHICS_OFFSET_VULKAN_PRESENT_LOAD_PASS,
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

#[path = "vulkan_present.rs"]
mod present;
pub(crate) use present::{CANVAS_CHILD_SYMBOL, WM_APP_CANVAS_SHOW};

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
    // bug-688 V6: the swapchain present (`vulkan_present.rs`). Resolved only when a
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
const SPIRV_VERTEX: &[u8] = include_bytes!("shaders/mfb_canvas.vert.spv");
const SPIRV_FRAGMENT: &[u8] = include_bytes!("shaders/mfb_canvas.frag.spv");

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

/// One member of a Vulkan create-info struct.
///
/// Vulkan's structs are the bulk of this backend, and writing each as a run of
/// `move_immediate`/`store_u64` would make every offset a bare number inside a wall
/// of emitter calls — unreadable, and unauditable in a file where nothing checks the
/// offsets and a wrong one hands garbage to a driver. So each struct is a *table*,
/// one row per member, naming the member beside its offset.
enum Field<'a> {
    /// A 32-bit value: an `sType`, an enum, a flags word, a count.
    U32(&'a str),
    /// The address of another stack slot — `sp + slot`.
    Addr(usize),
}

/// Zero `size` bytes at `sp + base`, then write each named member.
///
/// The zeroing is not tidiness. Vulkan reads `pNext` and `flags` on every struct, and
/// a reserved field left holding stack garbage is not ignored — so zeroing the whole
/// range and writing only the members the table names is what makes each table
/// *complete* rather than merely representative.
fn emit_struct(builder: &mut CodeBuilder, base: usize, size: usize, fields: &[(usize, Field)]) {
    emit_zero_range(builder, base, size);
    for (offset, field) in fields {
        match field {
            Field::U32(value) => {
                builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
                builder.emit(abi::store_u32(
                    abi::SCRATCH[0],
                    abi::stack_pointer(),
                    base + offset,
                ));
            }
            Field::Addr(slot) => {
                builder.emit(abi::add_immediate(
                    abi::SCRATCH[0],
                    abi::stack_pointer(),
                    *slot,
                ));
                builder.emit(abi::store_u64(
                    abi::SCRATCH[0],
                    abi::stack_pointer(),
                    base + offset,
                ));
            }
        }
    }
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

/// The per-draw payload, `uniform Draw` in both stages: the group offset — `dx` and `dy`
/// in 16.16 (plan-116-H), then (bug-735) their residual words, `ivec4 offset` — and
/// (plan-172-H) the inverse linear part of the groups' composed affine as four float32
/// bit patterns, `ivec4 inverse`, all zero for a draw its groups only translate. Eight
/// 32-bit words.
///
/// Thirty-two bytes against the 128 every implementation guarantees, so the size is not
/// the constraint — the constraint is that it is the *only* push constant, so its offset
/// is 0 and any future addition has to be appended rather than inserted, as the
/// residuals and the matrix were. The pipeline layout's range, the `vkCmdPushConstants`
/// size and the stack slot the values are staged in all read this one constant.
const PUSH_OFFSET_BYTES: usize = 32;

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

/// `canvas::vulkanBuild(where)` — build the device, the queue and the pipeline, once.
///
/// bug-996: this is the slow half of what `canvas::vulkanReady` used to be, and it is
/// never on a frame's critical path. `where` says which thread runs it
/// ([`vulkan_handoff`]'s `BUILD_*`): a thread of its own, started here and never
/// waited for; or the caller, when the program asked for the GPU by name; or, for the
/// builder thread's own entry, the caller with the claim already made. The state it
/// leaves in `GRAPHICS_OFFSET_VULKAN_READY`, and why the graphics thread can trust
/// it, are that module's.
///
/// The body touches no arena state (it runs on a thread that has none): its own
/// stack, the process-global graphics state, and the loader.
///
/// Picks the **first** physical device and its first graphics-capable queue family.
/// A machine with a discrete and an integrated GPU gets whichever the loader lists
/// first, which is the loader's own preference order — and for a renderer whose
/// output is checked against a software oracle within a tolerance, "which GPU" is not
/// a question this letter needs an opinion on.
pub(crate) fn emit_vulkan_build(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    place: &Operand,
) -> Result<(), String> {
    if !has_vulkan_backend(platform) {
        return Ok(());
    }
    let ordered = vulkan_handoff::orders_ready_word(platform);
    let unavailable = builder.label("vk_ready_unavailable");
    let done = builder.label("vk_ready_done");
    let build = builder.label("vk_ready_build");
    let family_head = builder.label("vk_family_head");
    let family_found = builder.label("vk_family_found");

    // **Parked before any scratch register is written.** `place` is a vreg, and the
    // allocator does not see the hand-assigned `SCRATCH` values this file keeps live,
    // so it may colour a vreg onto one of them (`emit_state_load`'s doc has the case
    // that did).
    let off_place = builder.allocate_stack_object("vk_place", 8);
    builder.emit(abi::store_u64(
        place.clone(),
        abi::stack_pointer(),
        off_place,
    ));

    let off_app_info = builder.allocate_stack_object("vk_app_info", APP_INFO_SIZE);
    let off_instance_info = builder.allocate_stack_object("vk_instance_info", INSTANCE_INFO_SIZE);
    let off_queue_info = builder.allocate_stack_object("vk_queue_info", QUEUE_INFO_SIZE);
    let off_device_info = builder.allocate_stack_object("vk_device_info", DEVICE_INFO_SIZE);
    let off_priority = builder.allocate_stack_object("vk_priority", 8);
    let off_devices = builder.allocate_stack_object("vk_devices", MAX_PHYSICAL_DEVICES * 8);
    let off_families = builder.allocate_stack_object(
        "vk_families",
        MAX_QUEUE_FAMILIES * QUEUE_FAMILY_PROPERTIES_SIZE,
    );
    let off_count = builder.allocate_stack_object("vk_count", 8);
    let off_out = builder.allocate_stack_object("vk_out", 8);
    let off_handle = builder.allocate_stack_object("vk_handle", 8);
    let off_fn = builder.allocate_stack_object("vk_fn", 8);
    let off_state = builder.allocate_stack_object("vk_state", 8);
    let off_index = builder.allocate_stack_object("vk_index", 8);

    state_base_into(builder, off_state);

    // The builder thread's own entry: the claim was made for it.
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_place,
    ));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        vulkan_handoff::BUILD_CLAIMED,
    ));
    builder.emit(abi::branch_eq(&build));

    // Anyone else claims it first, and leaves if it is already claimed: running, built
    // or failed. `SCRATCH[0]` still holds the state's address.
    vulkan_handoff::emit_claim(
        ordered,
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[2],
        &done,
        &mut builder.instructions,
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_place,
    ));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        vulkan_handoff::BUILD_ON_THIS_THREAD,
    ));
    builder.emit(abi::branch_eq(&build));
    emit_spawn_builder(builder, platform, platform_imports, off_state, &unavailable)?;
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&build));

    // handle = dlopen("libvulkan.so.1", RTLD_NOW)
    //        | LoadLibraryExA("vulkan-1.dll", NULL, 0)
    emit_open_library(builder, platform, platform_imports)?;
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&unavailable));
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_handle,
    ));

    // instance = vkCreateInstance(&createInfo, NULL, &instance)
    emit_struct(
        builder,
        off_app_info,
        APP_INFO_SIZE,
        &[
            (APP_INFO_STYPE, Field::U32(ST_APPLICATION_INFO)),
            (APP_INFO_API_VERSION, Field::U32(VK_API_VERSION_1_0)),
        ],
    );
    emit_struct(
        builder,
        off_instance_info,
        INSTANCE_INFO_SIZE,
        &[
            (INSTANCE_INFO_STYPE, Field::U32(ST_INSTANCE_CREATE_INFO)),
            (INSTANCE_INFO_APP_INFO, Field::Addr(off_app_info)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreateInstance",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    if present::has_present_path(platform) {
        // bug-688 V6: with the window-system extensions when the loader has them.
        present::emit_wsi_create_instance(builder, off_instance_info, off_out, off_fn, off_state);
    } else {
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
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&unavailable));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_INSTANCE,
        abi::SCRATCH[0],
    );

    // vkEnumeratePhysicalDevices(instance, &count, devices)
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &MAX_PHYSICAL_DEVICES.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_count,
    ));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkEnumeratePhysicalDevices",
        off_handle,
        off_fn,
        &unavailable,
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
        off_count,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_devices,
    ));
    emit_call_fn(builder, off_fn);
    // **Check the result before the count.** The count was pre-set to the array
    // capacity, because that is how Vulkan's enumerate-into-an-array form is told how
    // much room it has — so a call that fails without writing it leaves 8 there, and
    // reading the count alone would say "eight devices" on a machine with none.
    //
    // A negative `VkResult` is an error; `VK_INCOMPLETE` (5) is not — it means there
    // were more devices than the array, which is fine when any one will do.
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_lt(&unavailable));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&unavailable));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_devices,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::SCRATCH[0],
    );

    // vkGetPhysicalDeviceQueueFamilyProperties(phys, &count, families)
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &MAX_QUEUE_FAMILIES.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_count,
    ));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetPhysicalDeviceQueueFamilyProperties",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_families,
    ));
    emit_call_fn(builder, off_fn);

    // The first family with VK_QUEUE_GRAPHICS_BIT.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::label(&family_head));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_ge(&unavailable));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &QUEUE_FAMILY_PROPERTIES_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        off_families,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::load_u32(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        QUEUE_FAMILY_FLAGS,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[4],
        "Integer",
        QUEUE_GRAPHICS_BIT,
    ));
    builder.emit(abi::and_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[4],
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[2], "0"));
    builder.emit(abi::branch_ne(&family_found));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::branch(&family_head));

    builder.emit(abi::label(&family_found));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY,
        abi::SCRATCH[0],
    );

    // device = vkCreateDevice(phys, &deviceInfo, NULL, &device)
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FLOAT_ONE_BITS,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_priority,
    ));
    emit_struct(
        builder,
        off_queue_info,
        QUEUE_INFO_SIZE,
        &[
            (QUEUE_INFO_STYPE, Field::U32(ST_DEVICE_QUEUE_CREATE_INFO)),
            (QUEUE_INFO_COUNT, Field::U32("1")),
            (QUEUE_INFO_PRIORITIES, Field::Addr(off_priority)),
        ],
    );
    // The family index is a register, so it is written after the table.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_queue_info + QUEUE_INFO_FAMILY,
    ));
    emit_struct(
        builder,
        off_device_info,
        DEVICE_INFO_SIZE,
        &[
            (DEVICE_INFO_STYPE, Field::U32(ST_DEVICE_CREATE_INFO)),
            (DEVICE_INFO_QUEUE_COUNT, Field::U32("1")),
            (DEVICE_INFO_QUEUE_INFOS, Field::Addr(off_queue_info)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreateDevice",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    if present::has_present_path(platform) {
        // bug-688 V6: with `VK_KHR_swapchain` when the instance took the WSI ones.
        present::emit_wsi_create_device(builder, off_device_info, off_out, off_fn, off_state);
    } else {
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
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&unavailable));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DEVICE,
        abi::SCRATCH[0],
    );

    // vkGetDeviceQueue(device, family, 0, &queue)
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetDeviceQueue",
        off_handle,
        off_fn,
        &unavailable,
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
        GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
        abi::stack_pointer(),
        off_out,
    ));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_out,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&unavailable));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_QUEUE,
        abi::SCRATCH[0],
    );

    // The render pass, layout and pipeline, built once alongside the device — there
    // is no point reporting "ready" for a device that cannot record a frame.
    emit_vulkan_descriptors(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_out,
        &unavailable,
    )?;
    emit_vulkan_pipeline(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_out,
        &unavailable,
    )?;

    // Remember the library handle, then publish: the ready word is the last thing
    // this thread writes (`vulkan_handoff`).
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_handle,
    ));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_LIB,
        abi::SCRATCH[0],
    );
    emit_publish_state(builder, ordered, off_state, vulkan_handoff::VULKAN_BUILT);
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&unavailable));
    emit_publish_state(builder, ordered, off_state, vulkan_handoff::VULKAN_FAILED);

    builder.emit(abi::label(&done));
    Ok(())
}

/// Publish the build's answer: `state[READY] = value`, after every earlier store.
fn emit_publish_state(builder: &mut CodeBuilder, ordered: bool, off_state: usize, value: &str) {
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        off_state,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    vulkan_handoff::emit_publish_ready(
        ordered,
        abi::SCRATCH[3],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        &mut builder.instructions,
    );
}

/// Start the builder thread and hand it back to the system; branch to `failed` if the
/// thread library will not start one.
///
/// The thread is given the stack the graphics thread has (8 MiB, reserved lazily),
/// because until bug-996 the graphics thread is where this build ran: a driver's shader
/// compiler is not a small-stack kind of code. On POSIX that is the attribute object
/// `canvas::startGraphics` initialised, which is read-only from then on.
///
/// Nobody joins it. `pthread_detach` / `CloseHandle` gives up the only reference the
/// program holds, so the thread's bookkeeping goes when it returns.
fn emit_spawn_builder(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_state: usize,
    failed: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    const STACK_BYTES: &str = "8388608";
    if matches!(platform.family(), PlatformFamily::Windows) {
        // CreateThread(NULL, 8 MiB, entry, NULL, STACK_SIZE_PARAM_IS_A_RESERVATION, NULL)
        builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0"));
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", STACK_BYTES));
        push_symbol_address(
            &symbol,
            vulkan_handoff::VULKAN_BUILDER_SYMBOL,
            abi::c_arg(2),
            &mut builder.instructions,
            &mut builder.relocations,
        );
        builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
        emit_int_arg(builder, platform, 4, "65536");
        emit_int_arg(builder, platform, 5, "0");
        platform.emit_external_call(
            "CreateThread",
            &symbol,
            platform_imports,
            &mut builder.instructions,
            &mut builder.relocations,
        )?;
        builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
        builder.emit(abi::branch_eq(failed));
        builder.emit(abi::move_register(abi::c_arg(0), abi::c_return(0)));
        return platform.emit_external_call(
            "CloseHandle",
            &symbol,
            platform_imports,
            &mut builder.instructions,
            &mut builder.relocations,
        );
    }
    // pthread_create(&thread, &attr, entry, NULL)
    let off_thread = builder.allocate_stack_object("vk_builder_thread", 8);
    builder.emit(abi::load_u64(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_state,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::c_arg(1),
        GRAPHICS_OFFSET_ATTR,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_thread,
    ));
    push_symbol_address(
        &symbol,
        vulkan_handoff::VULKAN_BUILDER_SYMBOL,
        abi::c_arg(2),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    platform.emit_external_call(
        "pthread_create",
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(failed));
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_thread,
    ));
    platform.emit_external_call(
        "pthread_detach",
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )
}

/// `canvas::vulkanReady() AS Boolean` — is the pipeline built, now?
///
/// The Vulkan twin of `canvas::metalReady` as far as the renderer branch reads it, and
/// unlike it a question only: it builds nothing and waits for nothing (bug-996,
/// [`vulkan_handoff::emit_vulkan_is_ready`]). A target with no Vulkan backend answers
/// FALSE.
pub(crate) fn emit_vulkan_ready(builder: &mut CodeBuilder, platform: &dyn CodegenPlatform) {
    if !has_vulkan_backend(platform) {
        builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return;
    }
    let symbol = builder.current_symbol.clone();
    let state = builder.temporary_vreg();
    let value = builder.temporary_vreg();
    vulkan_handoff::emit_vulkan_is_ready(
        &symbol,
        vulkan_handoff::orders_ready_word(platform),
        abi::Reg::from(&state),
        abi::Reg::from(&value),
        &mut builder.instructions,
        &mut builder.relocations,
    );
}

/// Create the descriptor set that binds the polygon edge buffer, and the buffer.
///
/// **Two** bindings, one set, two descriptors — both `readonly buffer`s.
///
/// - `binding = 0`, fragment only: the edge/glyph buffer the fragment shader walks
///   when `kind` is `Polygon` or `Text`. It exists because those two payloads are
///   unbounded, which no per-draw transport can carry.
/// - `binding = 1`, **both stages**: the per-frame item buffer (plan-116-A). Both
///   stages, because the vertex stage reads `quad`/`surface` to place the corners and
///   the fragment stage reads everything else — the same two-stage visibility the
///   push-constant range it replaced had.
///
/// This runs *before* `emit_vulkan_pipeline`, not after, because the pipeline
/// **layout** names the set layout. It is also here rather than in
/// `emit_vulkan_target` because none of it depends on the surface size, so a resize
/// must not tear it down and rebuild it.
///
/// The buffer is host-visible and mapped once for its lifetime, the same way the
/// readback buffer is: it is rewritten every frame, so a map/unmap pair per frame
/// would cost more than the writes it brackets.
#[allow(clippy::too_many_arguments)]
fn emit_vulkan_descriptors(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    unavailable: &str,
) -> Result<(), String> {
    // An *array* of two bindings, so they must be contiguous — hence one allocation of
    // twice the size rather than two objects, whose relative placement is the
    // allocator's business and not something to bet a descriptor layout on.
    let off_binding = builder.allocate_stack_object("vk_binding", BINDING_SIZE * 2);
    let off_set_layout_info =
        builder.allocate_stack_object("vk_set_layout_info", SET_LAYOUT_INFO_SIZE);
    let off_pool_size = builder.allocate_stack_object("vk_pool_size", POOL_SIZE_SIZE);
    let off_desc_pool_info =
        builder.allocate_stack_object("vk_desc_pool_info", DESC_POOL_INFO_SIZE);
    let off_desc_alloc = builder.allocate_stack_object("vk_desc_alloc", DESC_ALLOC_INFO_SIZE);
    let off_layout_handle = builder.allocate_stack_object("vk_layout_handle", 8);
    let off_edge_info = builder.allocate_stack_object("vk_edge_info", BUFFER_INFO_SIZE);
    let off_reqs = builder.allocate_stack_object("vk_edge_reqs", MEMORY_REQS_SIZE);
    let off_alloc_info = builder.allocate_stack_object("vk_edge_alloc", ALLOC_INFO_SIZE);
    let off_properties =
        builder.allocate_stack_object("vk_edge_properties", MEMORY_PROPERTIES_SIZE);
    let off_type_index = builder.allocate_stack_object("vk_edge_type_index", 8);
    let off_type_bits = builder.allocate_stack_object("vk_edge_type_bits", 8);
    // Two of each, contiguous: `vkUpdateDescriptorSets` takes an array of writes, and
    // each write points at its own buffer info. One call, both bindings.
    let off_desc_buffer =
        builder.allocate_stack_object("vk_desc_buffer", DESC_BUFFER_INFO_SIZE * 2);
    let off_write_set = builder.allocate_stack_object("vk_write_set", WRITE_SET_SIZE * 2);

    // --- the set layout -----------------------------------------------------------
    emit_struct(
        builder,
        off_binding,
        BINDING_SIZE,
        &[
            (BINDING_INDEX, Field::U32("0")),
            (BINDING_TYPE, Field::U32(DESCRIPTOR_TYPE_STORAGE_BUFFER)),
            (BINDING_COUNT, Field::U32("1")),
            (BINDING_STAGES, Field::U32(SHADER_STAGE_FRAGMENT)),
        ],
    );
    emit_struct(
        builder,
        off_binding + BINDING_SIZE,
        BINDING_SIZE,
        &[
            (BINDING_INDEX, Field::U32("1")),
            (BINDING_TYPE, Field::U32(DESCRIPTOR_TYPE_STORAGE_BUFFER)),
            (BINDING_COUNT, Field::U32("1")),
            (BINDING_STAGES, Field::U32(SHADER_STAGE_VERTEX_AND_FRAGMENT)),
        ],
    );
    emit_struct(
        builder,
        off_set_layout_info,
        SET_LAYOUT_INFO_SIZE,
        &[
            (0, Field::U32(ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO)),
            (SET_LAYOUT_BINDING_COUNT, Field::U32("2")),
            (SET_LAYOUT_BINDINGS, Field::Addr(off_binding)),
        ],
    );
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateDescriptorSetLayout",
        off_handle,
        off_fn,
        off_state,
        off_set_layout_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_SET_LAYOUT,
        unavailable,
    )?;

    // --- the pool, and the one set allocated from it -------------------------------
    emit_struct(
        builder,
        off_pool_size,
        POOL_SIZE_SIZE,
        &[
            (POOL_SIZE_TYPE, Field::U32(DESCRIPTOR_TYPE_STORAGE_BUFFER)),
            // Two storage-buffer descriptors now — edges and items — still in one set.
            (POOL_SIZE_COUNT, Field::U32("2")),
        ],
    );
    emit_struct(
        builder,
        off_desc_pool_info,
        DESC_POOL_INFO_SIZE,
        &[
            (0, Field::U32(ST_DESCRIPTOR_POOL_CREATE_INFO)),
            (DESC_POOL_MAX_SETS, Field::U32("1")),
            (DESC_POOL_SIZE_COUNT, Field::U32("1")),
            (DESC_POOL_SIZES, Field::Addr(off_pool_size)),
        ],
    );
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateDescriptorPool",
        off_handle,
        off_fn,
        off_state,
        off_desc_pool_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_DESC_POOL,
        unavailable,
    )?;

    // `pSetLayouts` is an *array* of set-layout handles, so the handle has to be in
    // memory to be pointed at — it cannot be inlined into the struct.
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SET_LAYOUT,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_layout_handle,
    ));
    emit_struct(
        builder,
        off_desc_alloc,
        DESC_ALLOC_INFO_SIZE,
        &[
            (0, Field::U32(ST_DESCRIPTOR_SET_ALLOCATE_INFO)),
            (DESC_ALLOC_SET_COUNT, Field::U32("1")),
            (DESC_ALLOC_LAYOUTS, Field::Addr(off_layout_handle)),
        ],
    );
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_DESC_POOL,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_desc_alloc + DESC_ALLOC_POOL,
    ));
    // Not `emit_create_call`: `vkAllocateDescriptorSets` is (device, &info, &out) —
    // it has no allocator parameter, because the set comes from the pool.
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkAllocateDescriptorSets",
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
        off_desc_alloc,
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
        GRAPHICS_OFFSET_VULKAN_DESC_SET,
        abi::SCRATCH[0],
    );

    // --- the shared edge/glyph buffer ------------------------------------------------
    // One buffer, two regions: polygon edges from word 0, glyph coverage from
    // `VULKAN_GLYPH_BASE_WORDS`. A second buffer would need its own allocation, memory
    // type search, descriptor binding and upload, for data with exactly this one's
    // lifetime and exactly its access pattern.
    emit_struct(
        builder,
        off_edge_info,
        BUFFER_INFO_SIZE,
        &[
            (0, Field::U32(ST_BUFFER_CREATE_INFO)),
            (BUFFER_INFO_USAGE, Field::U32(BUFFER_USAGE_STORAGE)),
        ],
    );
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &VULKAN_BUFFER_BYTES.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_info + BUFFER_INFO_BYTES,
    ));
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateBuffer",
        off_handle,
        off_fn,
        off_state,
        off_edge_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER,
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
        GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER,
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
        &READBACK_MEMORY_CANDIDATES,
        GRAPHICS_OFFSET_VULKAN_EDGE_MEMORY,
        "vkBindBufferMemory",
        GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER,
        unavailable,
    )?;
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
        GRAPHICS_OFFSET_VULKAN_EDGE_MEMORY,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0")); // offset
    builder.emit(abi::move_immediate(
        abi::c_arg(3),
        "Integer",
        &VULKAN_BUFFER_BYTES.to_string(),
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
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        abi::SCRATCH[0],
    );

    // --- the per-frame item buffer (plan-116-A) --------------------------------------
    // A second buffer rather than a third region of the first, and that asymmetry is
    // deliberate. Edges and glyph coverage share one buffer because they have the same
    // element type (a raw `int` array) and the same reader (the fragment stage). The
    // item blocks are a *struct* array read by both stages, so sharing would mean one
    // `readonly buffer` whose element type is a union of the two — the shader would
    // have to index blocks as raw ints and reassemble them by hand, which is exactly
    // the packing mistake that produces a plausible wrong picture rather than a fault.
    //
    // Every step below mirrors the edge buffer's, reusing its scratch structs: they
    // describe one creation at a time and that one is finished.
    emit_struct(
        builder,
        off_edge_info,
        BUFFER_INFO_SIZE,
        &[
            (0, Field::U32(ST_BUFFER_CREATE_INFO)),
            (BUFFER_INFO_USAGE, Field::U32(BUFFER_USAGE_STORAGE)),
        ],
    );
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &VULKAN_ITEM_BUFFER_BYTES.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_info + BUFFER_INFO_BYTES,
    ));
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkCreateBuffer",
        off_handle,
        off_fn,
        off_state,
        off_edge_info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_ITEM_BUFFER,
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
        GRAPHICS_OFFSET_VULKAN_ITEM_BUFFER,
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
        &READBACK_MEMORY_CANDIDATES,
        GRAPHICS_OFFSET_VULKAN_ITEM_MEMORY,
        "vkBindBufferMemory",
        GRAPHICS_OFFSET_VULKAN_ITEM_BUFFER,
        unavailable,
    )?;
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
        GRAPHICS_OFFSET_VULKAN_ITEM_MEMORY,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0")); // offset
    builder.emit(abi::move_immediate(
        abi::c_arg(3),
        "Integer",
        &VULKAN_ITEM_BUFFER_BYTES.to_string(),
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
        GRAPHICS_OFFSET_VULKAN_ITEM_MAPPED,
        abi::SCRATCH[0],
    );

    // --- point the set at the buffers, once -----------------------------------------
    // Zero it first. `offset` sits between `buffer` and `range` and is not written
    // below, and a storage buffer's offset must be a multiple of
    // `minStorageBufferOffsetAlignment` — so leftover stack garbage there is not a
    // wrong picture, it is a rejected descriptor write and a blank frame.
    for (slot, buffer_offset, bytes, binding) in [
        (
            0usize,
            GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER,
            VULKAN_BUFFER_BYTES,
            "0",
        ),
        (
            1,
            GRAPHICS_OFFSET_VULKAN_ITEM_BUFFER,
            VULKAN_ITEM_BUFFER_BYTES,
            "1",
        ),
    ] {
        let info = off_desc_buffer + slot * DESC_BUFFER_INFO_SIZE;
        let write = off_write_set + slot * WRITE_SET_SIZE;
        emit_struct(builder, info, DESC_BUFFER_INFO_SIZE, &[]);
        emit_state_load(builder, off_state, buffer_offset, abi::SCRATCH[0]);
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            info + DESC_BUFFER_INFO_BUFFER,
        ));
        builder.emit(abi::move_immediate(
            abi::SCRATCH[0],
            "Integer",
            &bytes.to_string(),
        ));
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            info + DESC_BUFFER_INFO_RANGE,
        ));
        emit_struct(
            builder,
            write,
            WRITE_SET_SIZE,
            &[
                (0, Field::U32(ST_WRITE_DESCRIPTOR_SET)),
                (WRITE_SET_BINDING, Field::U32(binding)),
                (WRITE_SET_COUNT, Field::U32("1")),
                (WRITE_SET_TYPE, Field::U32(DESCRIPTOR_TYPE_STORAGE_BUFFER)),
                (WRITE_SET_BUFFER_INFO, Field::Addr(info)),
            ],
        );
        emit_state_load(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_DESC_SET,
            abi::SCRATCH[0],
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            write + WRITE_SET_DST,
        ));
    }
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkUpdateDescriptorSets",
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
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "2"));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_write_set,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_int_arg(builder, platform, 4, "0");
    emit_call_fn(builder, off_fn);
    Ok(())
}

/// Build the render pass, pipeline layout and graphics pipeline.
///
/// One pipeline, as on Metal, and the same shape: no vertex buffer (the vertex stage
/// synthesizes its four corners from `gl_VertexIndex`), no descriptor sets (the
/// 112-byte item block fits Vulkan's guaranteed 128-byte push-constant range), and a
/// premultiplied `over` blend against an sRGB attachment so the GPU does the encode
/// exactly where the software oracle's table does.
///
/// **Viewport and scissor are dynamic.** They are the only pipeline state that
/// depends on the surface size, so making them dynamic is what lets a resize reuse
/// the pipeline instead of rebuilding it — the Vulkan equivalent of the Metal path
/// reallocating only its texture.
///
/// Returns with the pipeline stored in the graphics state, or branches to
/// `unavailable`.
#[allow(clippy::too_many_arguments)]
fn emit_vulkan_pipeline(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    unavailable: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();

    let off_module_info = builder.allocate_stack_object("vk_module_info", SHADER_MODULE_INFO_SIZE);
    let off_vert_module = builder.allocate_stack_object("vk_vert_module", 8);
    let off_frag_module = builder.allocate_stack_object("vk_frag_module", 8);
    let off_layout_info = builder.allocate_stack_object("vk_layout_info", LAYOUT_INFO_SIZE);
    let off_set_layout_handle = builder.allocate_stack_object("vk_set_layout_handle", 8);
    let off_attachment = builder.allocate_stack_object("vk_attachment", ATTACHMENT_SIZE);
    let off_attachment_ref =
        builder.allocate_stack_object("vk_attachment_ref", ATTACHMENT_REF_SIZE);
    let off_subpass = builder.allocate_stack_object("vk_subpass", SUBPASS_SIZE);
    let off_pass_info = builder.allocate_stack_object("vk_pass_info", RENDER_PASS_INFO_SIZE);
    let off_stages = builder.allocate_stack_object("vk_stages", STAGE_INFO_SIZE * 2);
    let off_spec_info = builder.allocate_stack_object("vk_spec_info", SPEC_INFO_SIZE);
    let off_spec_entries = builder.allocate_stack_object(
        "vk_spec_entries",
        SPEC_ENTRY_SIZE * VULKAN_SPEC_CONSTANTS.len(),
    );
    let off_spec_data =
        builder.allocate_stack_object("vk_spec_data", 4 * VULKAN_SPEC_CONSTANTS.len());
    let off_vertex_input = builder.allocate_stack_object("vk_vertex_input", VERTEX_INPUT_INFO_SIZE);
    let off_input_assembly =
        builder.allocate_stack_object("vk_input_assembly", INPUT_ASSEMBLY_INFO_SIZE);
    let off_viewport_info = builder.allocate_stack_object("vk_viewport_info", VIEWPORT_INFO_SIZE);
    let off_raster = builder.allocate_stack_object("vk_raster", RASTER_INFO_SIZE);
    let off_multisample = builder.allocate_stack_object("vk_multisample", MULTISAMPLE_INFO_SIZE);
    let off_blend_attachment =
        builder.allocate_stack_object("vk_blend_attachment", BLEND_ATTACHMENT_SIZE);
    let off_blend_info = builder.allocate_stack_object("vk_blend_info", BLEND_INFO_SIZE);
    let off_dynamic_states = builder.allocate_stack_object("vk_dynamic_states", 8);
    let off_dynamic_info = builder.allocate_stack_object("vk_dynamic_info", DYNAMIC_INFO_SIZE);
    let off_pipeline_info = builder.allocate_stack_object("vk_pipeline_info", PIPELINE_INFO_SIZE);

    // --- the two shader modules ---------------------------------------------------
    for (blob_symbol, blob_len, module_slot) in [
        (vertex_spirv_symbol(), SPIRV_VERTEX.len(), off_vert_module),
        (
            fragment_spirv_symbol(),
            SPIRV_FRAGMENT.len(),
            off_frag_module,
        ),
    ] {
        emit_struct(
            builder,
            off_module_info,
            SHADER_MODULE_INFO_SIZE,
            &[
                (
                    SHADER_MODULE_STYPE,
                    Field::U32(ST_SHADER_MODULE_CREATE_INFO),
                ),
                (SHADER_MODULE_CODE_SIZE, Field::U32(&blob_len.to_string())),
            ],
        );
        emit_data_address(
            &symbol,
            abi::SCRATCH[0],
            &blob_symbol,
            &mut builder.instructions,
            &mut builder.relocations,
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_module_info + SHADER_MODULE_CODE,
        ));
        emit_dlsym(
            builder,
            platform,
            platform_imports,
            "vkCreateShaderModule",
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
            off_module_info,
        ));
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
        builder.emit(abi::add_immediate(
            abi::c_arg(3),
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
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            module_slot,
        ));
    }

    // --- pipeline layout: one set, and NO push-constant range ---------------------
    // plan-116-A moved the item block into `binding = 1` of the set above, so neither
    // shader declares a push constant any more. The range is not merely unused — a
    // layout that declares bytes no stage consumes is a layout the validation layers
    // flag, and it is the one line that would keep the 128-byte ceiling alive in the
    // API even after the shaders stopped caring about it.
    // `pSetLayouts` is an array, so the set-layout handle has to be somewhere
    // addressable — the same reason `vkAllocateDescriptorSets` parks it.
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_SET_LAYOUT,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_set_layout_handle,
    ));
    // plan-116-H: one push-constant range, consumed by BOTH stages. `emit_struct` zeroes
    // the struct first, so the fields not named here are already 0 — which is what
    // `offset` must be, this being the only range.
    let off_push_range = builder.allocate_stack_object("vk_push_range", PUSH_RANGE_SIZE);
    emit_struct(
        builder,
        off_push_range,
        PUSH_RANGE_SIZE,
        &[
            (
                PUSH_RANGE_STAGES,
                Field::U32(SHADER_STAGE_VERTEX_AND_FRAGMENT),
            ),
            (PUSH_RANGE_OFFSET, Field::U32("0")),
            (PUSH_RANGE_BYTES, Field::U32(&PUSH_OFFSET_BYTES.to_string())),
        ],
    );
    emit_struct(
        builder,
        off_layout_info,
        LAYOUT_INFO_SIZE,
        &[
            (
                LAYOUT_INFO_STYPE,
                Field::U32(ST_PIPELINE_LAYOUT_CREATE_INFO),
            ),
            (LAYOUT_INFO_SET_COUNT, Field::U32("1")),
            (LAYOUT_INFO_SETS, Field::Addr(off_set_layout_handle)),
            (LAYOUT_INFO_RANGE_COUNT, Field::U32("1")),
            (LAYOUT_INFO_RANGES, Field::Addr(off_push_range)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreatePipelineLayout",
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
        off_layout_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
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
        GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
        abi::SCRATCH[0],
    );

    // --- render pass: one sRGB colour attachment, cleared and stored --------------
    emit_struct(
        builder,
        off_attachment,
        ATTACHMENT_SIZE,
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
                Field::U32(IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL),
            ),
        ],
    );
    emit_struct(
        builder,
        off_attachment_ref,
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
            (SUBPASS_COLOR_ATTACHMENTS, Field::Addr(off_attachment_ref)),
        ],
    );
    emit_struct(
        builder,
        off_pass_info,
        RENDER_PASS_INFO_SIZE,
        &[
            (RENDER_PASS_STYPE, Field::U32(ST_RENDER_PASS_CREATE_INFO)),
            (RENDER_PASS_ATTACHMENT_COUNT, Field::U32("1")),
            (RENDER_PASS_ATTACHMENTS, Field::Addr(off_attachment)),
            (RENDER_PASS_SUBPASS_COUNT, Field::U32("1")),
            (RENDER_PASS_SUBPASSES, Field::Addr(off_subpass)),
        ],
    );
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreateRenderPass",
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
        off_pass_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
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
        GRAPHICS_OFFSET_VULKAN_RENDER_PASS,
        abi::SCRATCH[0],
    );

    // --- the fragment stage's region bases (bug-688) -------------------------------
    // The shader's GLYPH_BASE / GRADIENT_BASE / BAND_BASE are specialization constants,
    // so the one definition of each is the Rust constant and nothing in the checked-in
    // SPIR-V spells a layout number. One 32-bit value per constant, in the order of
    // `VULKAN_SPEC_CONSTANTS`, each entry naming its own slice of the data.
    let mut entry_fields = Vec::new();
    let mut data_fields = Vec::new();
    let spec_values: Vec<(String, String, String)> = VULKAN_SPEC_CONSTANTS
        .iter()
        .enumerate()
        .map(|(n, (id, value))| (id.to_string(), (n * 4).to_string(), value.to_string()))
        .collect();
    for (n, (id, offset, value)) in spec_values.iter().enumerate() {
        let entry = n * SPEC_ENTRY_SIZE;
        entry_fields.push((entry + SPEC_ENTRY_ID, Field::U32(id)));
        entry_fields.push((entry + SPEC_ENTRY_OFFSET, Field::U32(offset)));
        entry_fields.push((entry + SPEC_ENTRY_BYTES, Field::U32("4")));
        data_fields.push((n * 4, Field::U32(value)));
    }
    emit_struct(
        builder,
        off_spec_entries,
        SPEC_ENTRY_SIZE * VULKAN_SPEC_CONSTANTS.len(),
        &entry_fields,
    );
    emit_struct(
        builder,
        off_spec_data,
        4 * VULKAN_SPEC_CONSTANTS.len(),
        &data_fields,
    );
    let spec_count = VULKAN_SPEC_CONSTANTS.len().to_string();
    let spec_bytes = (4 * VULKAN_SPEC_CONSTANTS.len()).to_string();
    emit_struct(
        builder,
        off_spec_info,
        SPEC_INFO_SIZE,
        &[
            (SPEC_INFO_COUNT, Field::U32(&spec_count)),
            (SPEC_INFO_ENTRIES, Field::Addr(off_spec_entries)),
            (SPEC_INFO_DATA_SIZE, Field::U32(&spec_bytes)),
            (SPEC_INFO_DATA, Field::Addr(off_spec_data)),
        ],
    );

    // --- the pipeline -------------------------------------------------------------
    for (index, stage, module_slot) in [
        (0usize, SHADER_STAGE_VERTEX, off_vert_module),
        (1, SHADER_STAGE_FRAGMENT, off_frag_module),
    ] {
        let base = off_stages + index * STAGE_INFO_SIZE;
        emit_struct(
            builder,
            base,
            STAGE_INFO_SIZE,
            &[
                (
                    STAGE_INFO_STYPE,
                    Field::U32(ST_PIPELINE_SHADER_STAGE_CREATE_INFO),
                ),
                (STAGE_INFO_STAGE, Field::U32(stage)),
            ],
        );
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            module_slot,
        ));
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            base + STAGE_INFO_MODULE,
        ));
        emit_data_address(
            &symbol,
            abi::SCRATCH[0],
            &entry_name_symbol(),
            &mut builder.instructions,
            &mut builder.relocations,
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            base + STAGE_INFO_NAME,
        ));
        if stage == SHADER_STAGE_FRAGMENT {
            emit_struct(
                builder,
                base + STAGE_INFO_SPECIALIZATION,
                8,
                &[(0, Field::Addr(off_spec_info))],
            );
        }
    }

    emit_struct(
        builder,
        off_vertex_input,
        VERTEX_INPUT_INFO_SIZE,
        &[(0, Field::U32(ST_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO))],
    );
    emit_struct(
        builder,
        off_input_assembly,
        INPUT_ASSEMBLY_INFO_SIZE,
        &[
            (0, Field::U32(ST_PIPELINE_INPUT_ASSEMBLY_STATE_CREATE_INFO)),
            (INPUT_ASSEMBLY_TOPOLOGY, Field::U32(TOPOLOGY_TRIANGLE_STRIP)),
        ],
    );
    emit_struct(
        builder,
        off_viewport_info,
        VIEWPORT_INFO_SIZE,
        &[
            (0, Field::U32(ST_PIPELINE_VIEWPORT_STATE_CREATE_INFO)),
            (VIEWPORT_INFO_VIEWPORT_COUNT, Field::U32("1")),
            (VIEWPORT_INFO_SCISSOR_COUNT, Field::U32("1")),
        ],
    );
    emit_struct(
        builder,
        off_raster,
        RASTER_INFO_SIZE,
        &[
            (0, Field::U32(ST_PIPELINE_RASTERIZATION_STATE_CREATE_INFO)),
            (RASTER_POLYGON_MODE, Field::U32(POLYGON_MODE_FILL)),
            (RASTER_CULL_MODE, Field::U32(CULL_MODE_NONE)),
            (RASTER_FRONT_FACE, Field::U32(FRONT_FACE_CCW)),
            (RASTER_LINE_WIDTH, Field::U32(FLOAT_ONE_BITS)),
        ],
    );
    emit_struct(
        builder,
        off_multisample,
        MULTISAMPLE_INFO_SIZE,
        &[
            (0, Field::U32(ST_PIPELINE_MULTISAMPLE_STATE_CREATE_INFO)),
            (MULTISAMPLE_SAMPLES, Field::U32(SAMPLE_COUNT_1)),
        ],
    );
    // The blend attachment is filled per MODE inside the pipeline loop below
    // (plan-116-B); everything else about the four pipelines is identical, so only
    // this struct and the destination state slot vary.
    emit_struct(
        builder,
        off_blend_info,
        BLEND_INFO_SIZE,
        &[
            (0, Field::U32(ST_PIPELINE_COLOR_BLEND_STATE_CREATE_INFO)),
            (BLEND_INFO_ATTACHMENT_COUNT, Field::U32("1")),
            (BLEND_INFO_ATTACHMENTS, Field::Addr(off_blend_attachment)),
        ],
    );
    // VK_DYNAMIC_STATE_VIEWPORT (0) and VK_DYNAMIC_STATE_SCISSOR (1).
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_dynamic_states,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_dynamic_states + 4,
    ));
    emit_struct(
        builder,
        off_dynamic_info,
        DYNAMIC_INFO_SIZE,
        &[
            (0, Field::U32(ST_PIPELINE_DYNAMIC_STATE_CREATE_INFO)),
            (DYNAMIC_INFO_COUNT, Field::U32("2")),
            (DYNAMIC_INFO_STATES, Field::Addr(off_dynamic_states)),
        ],
    );

    // --- one pipeline per blend mode (plan-116-B) ----------------------------------
    // A blend mode is per-PIPELINE state on this API: it lives in
    // `VkPipelineColorBlendAttachmentState`, which is baked into the pipeline. So
    // "per-item blend" means four pipelines chosen per draw, not a shader branch.
    //
    // The factor pairs, on a PREMULTIPLIED source against an sRGB attachment (so the
    // hardware blends in linear, which is where the oracle defines the modes):
    //
    //   Normal    One / OneMinusSrcAlpha    S + D(1-a)
    //   Multiply  DstColor / OneMinusSrcAlpha    S·D + D(1-a)  = D + a(Cs·D - D)
    //   Screen    One / OneMinusSrcColor    S + D(1-S)    = D + a·Cs(1-D)
    //   Add       One / One                 S + D
    //
    // Each expands to exactly the oracle's equation for that mode — worked through in
    // the plan's §2 and confirmed by the reference image, not assumed.
    let modes = [
        (
            0usize,
            BLEND_FACTOR_ONE,
            BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
            GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES,
        ),
        (
            1,
            BLEND_FACTOR_DST_COLOR,
            BLEND_FACTOR_ONE_MINUS_SRC_ALPHA,
            GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES + 8,
        ),
        (
            2,
            BLEND_FACTOR_ONE,
            BLEND_FACTOR_ONE_MINUS_SRC_COLOR,
            GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES + 16,
        ),
        (
            3,
            BLEND_FACTOR_ONE,
            BLEND_FACTOR_ONE,
            GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES + 24,
        ),
    ];
    // The frame path indexes the pipeline array by the blend tag with no bounds check,
    // so a table shorter than the variant set binds a neighbouring state slot as a
    // pipeline handle. Tying the literal to the constant is what makes adding a
    // `BlendMode` variant fail here rather than in a frame.
    assert_eq!(
        modes.len(),
        BLEND_MODE_COUNT,
        "one pipeline per BlendMode variant"
    );
    for (mode, src_factor, dst_factor, slot) in modes {
        // The alpha channel keeps `One`/`OneMinusSrcAlpha` for every mode. The modes
        // are defined on COLOUR; the surface's alpha is written 255 everywhere by the
        // oracle, and a mode that also rewrote alpha would make the two disagree about
        // a channel neither is trying to blend.
        emit_struct(
            builder,
            off_blend_attachment,
            BLEND_ATTACHMENT_SIZE,
            &[
                (BLEND_ENABLE, Field::U32("1")),
                (BLEND_SRC_COLOR, Field::U32(src_factor)),
                (BLEND_DST_COLOR, Field::U32(dst_factor)),
                (BLEND_COLOR_OP, Field::U32(BLEND_OP_ADD)),
                (BLEND_SRC_ALPHA, Field::U32(BLEND_FACTOR_ONE)),
                (
                    BLEND_DST_ALPHA,
                    Field::U32(BLEND_FACTOR_ONE_MINUS_SRC_ALPHA),
                ),
                (BLEND_ALPHA_OP, Field::U32(BLEND_OP_ADD)),
                (BLEND_WRITE_MASK, Field::U32(COLOR_COMPONENT_RGBA)),
            ],
        );
        emit_struct(
            builder,
            off_pipeline_info,
            PIPELINE_INFO_SIZE,
            &[
                (0, Field::U32(ST_GRAPHICS_PIPELINE_CREATE_INFO)),
                (PIPELINE_STAGE_COUNT, Field::U32("2")),
                (PIPELINE_STAGES, Field::Addr(off_stages)),
                (PIPELINE_VERTEX_INPUT, Field::Addr(off_vertex_input)),
                (PIPELINE_INPUT_ASSEMBLY, Field::Addr(off_input_assembly)),
                (PIPELINE_VIEWPORT, Field::Addr(off_viewport_info)),
                (PIPELINE_RASTER, Field::Addr(off_raster)),
                (PIPELINE_MULTISAMPLE, Field::Addr(off_multisample)),
                (PIPELINE_COLOR_BLEND, Field::Addr(off_blend_info)),
                (PIPELINE_DYNAMIC, Field::Addr(off_dynamic_info)),
            ],
        );
        emit_state_load(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
            abi::SCRATCH[0],
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_pipeline_info + PIPELINE_LAYOUT,
        ));
        emit_state_load(
            builder,
            off_state,
            GRAPHICS_OFFSET_VULKAN_RENDER_PASS,
            abi::SCRATCH[0],
        );
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_pipeline_info + PIPELINE_RENDER_PASS,
        ));

        emit_dlsym(
            builder,
            platform,
            platform_imports,
            "vkCreateGraphicsPipelines",
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
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0")); // no cache
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "1")); // one pipeline
        builder.emit(abi::add_immediate(
            abi::c_arg(3),
            abi::stack_pointer(),
            off_pipeline_info,
        ));
        emit_int_arg(builder, platform, 4, "0");
        emit_addr_arg(builder, platform, 5, off_out);
        emit_call_fn(builder, off_fn);
        builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
        builder.emit(abi::branch_ne(unavailable));
        builder.emit(abi::load_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_out,
        ));
        emit_state_store(builder, off_state, slot, abi::SCRATCH[0]);
        // `Normal` lands in the legacy slot too — see `…_PIPELINE_MODES`' doc. The
        // readiness check and the once-per-frame bind both read that one.
        if mode == 0 {
            emit_state_store(
                builder,
                off_state,
                GRAPHICS_OFFSET_VULKAN_PIPELINE,
                abi::SCRATCH[0],
            );
        }
    }
    Ok(())
}

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
fn emit_vulkan_target(
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

/// `vkCreateX(device, &info, NULL, &out)` then store `out` into the state block.
#[allow(clippy::too_many_arguments)]
fn emit_create_call(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    name: &str,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_info: usize,
    off_out: usize,
    state_slot: usize,
    unavailable: &str,
) -> Result<(), String> {
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
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(3),
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
    emit_state_store(builder, off_state, state_slot, abi::SCRATCH[0]);
    Ok(())
}

/// `vkX(device)` — the one-argument device calls.
#[allow(clippy::too_many_arguments)]
fn emit_device_call_1(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    name: &str,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    unavailable: &str,
) -> Result<(), String> {
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
    emit_call_fn(builder, off_fn);
    Ok(())
}

/// Allocate memory for the requirements at `off_reqs` and bind it to `object`.
#[allow(clippy::too_many_arguments)]
fn emit_allocate_and_bind(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_reqs: usize,
    off_alloc_info: usize,
    off_properties: usize,
    off_type_index: usize,
    off_type_bits: usize,
    off_out: usize,
    required: &[&str],
    memory_slot: usize,
    bind_name: &str,
    object_slot: usize,
    unavailable: &str,
) -> Result<(), String> {
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_reqs + MEMORY_REQS_TYPE_BITS,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_type_bits,
    ));
    emit_pick_memory_type(
        builder,
        platform,
        platform_imports,
        off_handle,
        off_fn,
        off_state,
        off_properties,
        off_type_index,
        off_type_bits,
        required,
        unavailable,
    )?;
    emit_struct(
        builder,
        off_alloc_info,
        ALLOC_INFO_SIZE,
        &[(0, Field::U32(ST_MEMORY_ALLOCATE_INFO))],
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_reqs + MEMORY_REQS_BYTES,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_alloc_info + ALLOC_INFO_BYTES,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_type_index,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_alloc_info + ALLOC_INFO_TYPE_INDEX,
    ));
    emit_create_call(
        builder,
        platform,
        platform_imports,
        "vkAllocateMemory",
        off_handle,
        off_fn,
        off_state,
        off_alloc_info,
        off_out,
        memory_slot,
        unavailable,
    )?;
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        bind_name,
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
    emit_state_load(builder, off_state, object_slot, abi::c_arg(1));
    emit_state_load(builder, off_state, memory_slot, abi::c_arg(2));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_call_fn(builder, off_fn);
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(unavailable));
    Ok(())
}

/// Choose a memory type satisfying `type_bits` and `required`, leaving its index in
/// `off_index`.
///
/// The standard Vulkan scan: the requirements report a bitmask of *acceptable* type
/// indices, and the device reports what each type can do; the answer is the first
/// index that is in the mask and has every required property.
///
/// A miss branches to `unavailable`. That is not a machine without the memory —
/// Vulkan guarantees at least one `DEVICE_LOCAL` type and at least one
/// `HOST_VISIBLE | HOST_COHERENT` type — so a miss means the mask and the
/// requirement disagree, which is a caller bug and should fail the same way a
/// missing device does rather than silently allocating the wrong kind.
#[allow(clippy::too_many_arguments)]
fn emit_pick_memory_type(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_properties: usize,
    off_index: usize,
    off_type_bits: usize,
    required: &[&str],
    unavailable: &str,
) -> Result<(), String> {
    let found = builder.label("vk_memtype_found");

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkGetPhysicalDeviceMemoryProperties",
        off_handle,
        off_fn,
        unavailable,
    )?;
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PHYSICAL,
        abi::c_arg(0),
    );
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_properties,
    ));
    emit_call_fn(builder, off_fn);

    // bug-688 V5: candidates are tried in order, each a full scan of the device's
    // memory types; the first candidate any allowed type satisfies wins. A caller
    // that wants a faster type when the device has one (the readback buffer asks
    // for HOST_CACHED first) lists it ahead of its minimum.
    for (i, flags) in required.iter().enumerate() {
        let not_found = if i + 1 == required.len() {
            unavailable.to_string()
        } else {
            builder.label("vk_memtype_fallback")
        };
        emit_memtype_scan(
            builder,
            off_properties,
            off_index,
            off_type_bits,
            flags,
            &found,
            &not_found,
        );
        if i + 1 != required.len() {
            builder.emit(abi::label(&not_found));
        }
    }
    builder.emit(abi::label(&found));
    Ok(())
}

/// One pass over the device's memory types for [`emit_pick_memory_type`]: branch to
/// `found` with the matching index in `off_index`, or to `not_found` when no type
/// allowed by `off_type_bits` has every property in `required`.
fn emit_memtype_scan(
    builder: &mut CodeBuilder,
    off_properties: usize,
    off_index: usize,
    off_type_bits: usize,
    required: &str,
    found: &str,
    not_found: &str,
) {
    let head = builder.label("vk_memtype_head");
    let next = builder.label("vk_memtype_next");
    builder.emit(abi::move_immediate(abi::SCRATCH[5], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        off_index,
    ));

    builder.emit(abi::label(&head));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::load_u32(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_properties + MEMORY_PROPERTIES_TYPE_COUNT,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[5], abi::SCRATCH[6]));
    builder.emit(abi::branch_ge(not_found));

    // Is this index in the requirements' mask?
    builder.emit(abi::load_u32(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_type_bits,
    ));
    builder.emit(abi::shift_right_variable(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[7], "Integer", "1"));
    builder.emit(abi::and_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[6], "0"));
    builder.emit(abi::branch_eq(&next));

    // Does it have every required property?
    builder.emit(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &MEMORY_TYPE_STRIDE.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[5],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_properties + MEMORY_PROPERTIES_TYPES,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::load_u32(abi::SCRATCH[6], abi::SCRATCH[6], 0));
    builder.emit(abi::move_immediate(abi::SCRATCH[7], "Integer", required));
    builder.emit(abi::and_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[6],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[6], abi::SCRATCH[7]));
    builder.emit(abi::branch_eq(found));

    builder.emit(abi::label(&next));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[5], abi::SCRATCH[5], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        off_index,
    ));
    builder.emit(abi::branch(&head));
}

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

/// Fill the 128-byte item block at `sp + off_item` from the geometry header whose
/// address is in `SCRATCH[0]`.
///
/// The Vulkan-flavoured twin of the Metal emitter in
/// `target/macos_aarch64/app/metal.rs`. The two emit into different IRs — one builds
/// a standalone `CodeFunction` with `Asm`, the other writes into an `abi_function`
/// body through `CodeBuilder` — but they write the *same* block, because the layout
/// and the header slots are one definition in `runtime/canvas/mod.rs`. A change to
/// the contract therefore cannot move one backend without moving the other.
///
/// Positions narrow to 16.16 fixed point; colours cross as the whole 0–255 values
/// the header already stores, so nothing rounds a colour.
fn emit_item_block(
    builder: &mut CodeBuilder,
    off_item: usize,
    off_width: usize,
    off_height: usize,
) {
    let header = abi::SCRATCH[0];
    let scale = abi::FP_SCRATCH[0];
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[1]));

    // The 16.16 fields no distance reads: the bounds and the gradient axis.
    for (item_offset, slots) in [
        (
            ITEM_OFFSET_QUAD,
            [
                HEADER_BOUNDS,
                HEADER_BOUNDS + 1,
                HEADER_BOUNDS + 2,
                HEADER_BOUNDS + 3,
            ],
        ),
        // plan-116-F: the gradient's axis, four consecutive header slots narrowing to
        // 16.16 exactly like the bounds and the arc caps.
        (
            ITEM_OFFSET_GRADIENT,
            [
                HEADER_GRADIENT_FROM_X,
                HEADER_GRADIENT_FROM_X + 1,
                HEADER_GRADIENT_FROM_X + 2,
                HEADER_GRADIENT_FROM_X + 3,
            ],
        ),
    ] {
        for (index, slot) in slots.into_iter().enumerate() {
            builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
            builder.emit(abi::float_multiply_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                scale,
            ));
            builder.emit(abi::float_round_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            builder.emit(abi::store_u32(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                off_item + item_offset + index * 4,
            ));
        }
    }
    // bug-1083: how much of that box is rim, 16.16 like the box, in the two words the
    // vertex stage reads it from under a group matrix (`ITEM_RIM_WORDS`).
    for (index, word) in crate::codegen::runtime::canvas::ITEM_RIM_WORDS
        .into_iter()
        .enumerate()
    {
        let slot = crate::codegen::runtime::canvas::HEADER_RIM + index;
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + word,
        ));
    }

    // bug-735: every field a distance reads -- the shape parameters, the radius and
    // stroke half-width, an arc's sweep endpoints, an ellipse's rotation and the clip --
    // as its 16.16 word and, in the block's residual `ivec4`s, its residual. The twin of
    // the loop in Metal's `emit_item_block`, from the same table.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[3],
        abi::SCRATCH[1],
    ));
    for (slot, word, residual) in ITEM_RESIDUAL_FIELDS {
        for instruction in
            emit_item_residual_field(header, slot, off_item + word, off_item + residual)
        {
            builder.emit(instruction);
        }
    }
    let positive = builder.label("vk_item_stroke_positive");
    let signed = builder.label("vk_item_stroke_done");
    for instruction in emit_item_stroke_sign(header, off_item, &positive, &signed) {
        builder.emit(instruction);
    }
    // bug-735: `arc.x` = 0 and an arc's `reflex` flag in `arc.y`, decided in f64.
    let set = builder.label("vk_item_reflex_set");
    let done = builder.label("vk_item_reflex_done");
    for instruction in emit_item_arc_flags(header, off_item, &set, &done) {
        builder.emit(instruction);
    }
    // plan-163-A: a polygon's fill rule over that word, which only an arc reads.
    let rule_done = builder.label("vk_item_rule_done");
    for instruction in emit_item_polygon_rule(header, off_item, &rule_done) {
        builder.emit(instruction);
    }

    // Both colours, as whole numbers.
    for (item_offset, first) in [
        (ITEM_OFFSET_FILL, HEADER_FILL_R),
        (ITEM_OFFSET_STROKE, HEADER_STROKE_R),
    ] {
        for channel in 0..4 {
            builder.emit(abi::load_double(
                abi::FP_SCRATCH[1],
                header,
                (first + channel) * 8,
            ));
            builder.emit(abi::float_convert_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
            builder.emit(abi::store_u32(
                abi::SCRATCH[1],
                abi::stack_pointer(),
                off_item + item_offset + channel * 4,
            ));
        }
    }

    // misc = { kind, radius, strokeHalf, edgeCount }: the two whole numbers here, the
    // two 16.16 ones with their residuals above.
    for (index, slot) in [(0usize, HEADER_KIND), (3, HEADER_AUX0)] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_MISC + index * 4,
        ));
    }

    for (index, parked) in [off_width, off_height].into_iter().enumerate() {
        builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), parked));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_SURFACE + index * 4,
        ));
    }

    // The inverse transform (plan-116-C). The header already holds these as float32
    // BIT PATTERNS — `__canvas_float32Bits` narrowed them once, in MFBASIC, because
    // this assembler has no double→single convert — so the emitter's whole job is a
    // whole-number read and a 32-bit store. Seven slots: `ia..ity` then the flag.
    for (index, slot) in [
        HEADER_TRANSFORM_IA,
        HEADER_TRANSFORM_IB,
        HEADER_TRANSFORM_IC,
        HEADER_TRANSFORM_ID,
        HEADER_TRANSFORM_ITX,
        HEADER_TRANSFORM_ITY,
        HEADER_HAS_TRANSFORM,
    ]
    .into_iter()
    .enumerate()
    {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_TRANSFORM + index * 4,
        ));
    }

    // plan-163-D: a radial gradient's own inverse -- six float32 bit patterns and the
    // flag, header slots 57-63 -- word for word into `gradInv0`/`gradInv1`, the
    // transform's layout; and its focal offset and radius narrowed to 16.16 (`gradFocal`).
    for index in 0..7 {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            header,
            (HEADER_GRADIENT_INV + index) * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_GRADIENT_INV + index * 4,
        ));
    }
    debug_assert_eq!(HEADER_GRADIENT_INV + 6, HEADER_GRADIENT_HAS_INV);
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[2],
        abi::SCRATCH[1],
    ));
    for index in 0..3 {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            header,
            (HEADER_GRADIENT_FOCAL_X + index) * 8,
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_GRADIENT_FOCAL + index * 4,
        ));
    }

    // The blend tag, a whole 0..3 beside the surface size (plan-116-B). `Normal` is 0,
    // so an item that never set `Paint.blend` writes the value the pipeline it selects
    // has always had.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_BLEND * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_SURFACE + ITEM_SURFACE_BLEND,
    ));

    // The cap tag, a whole 0 or 1 in the per-kind block's last free word (plan-116-D).
    // Copied for every kind rather than under a `Line`/`Arc` branch: it is one store,
    // and the branch would cost more than it saves while giving the word two meanings
    // depending on how the item got here.
    // plan-116-F: the gradient's kind, a whole 0 or 1 in the block's last spare word.
    // plan-163-D: packed with the spread and the stroke apply, `kind + 2 * spread + 8 *
    // apply` (`ITEM_SURFACE_GRADIENT_KIND`), every term a small whole number.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_KIND * 8,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[2],
        header,
        HEADER_GRADIENT_SPREAD * 8,
    ));
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[2],
        header,
        HEADER_GRADIENT_APPLY * 8,
    ));
    for _ in 0..3 {
        builder.emit(abi::float_add_d(
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
            abi::FP_SCRATCH[2],
        ));
    }
    builder.emit(abi::float_add_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[2],
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_SURFACE + ITEM_SURFACE_GRADIENT_KIND,
    ));

    builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, HEADER_CAP * 8));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ARC + ITEM_ARC_CAP,
    ));
}

/// Append this item's polygon edges to the frame's edge buffer and record where they
/// landed.
///
/// **Why an offset rather than a rebind.** A Vulkan command buffer is recorded now
/// and executed later, all at once. There is one edge buffer for the frame, so
/// rewriting it per item — or rebinding the same buffer per item — would give every
/// polygon whatever the *last* one wrote. Each polygon therefore takes a slice and
/// carries its start index in its own item block, which is per-item. Metal has the
/// opposite property and needs none of this: `setFragmentBytes:` copies the bytes
/// into the command buffer at record time.
///
/// The geometry cache stores each edge as `x0, y0, dx, dy, invLenSq` doubles; the
/// shader wants the two endpoints in 16.16, and recomputes the rest. `invLenSq` is
/// the one quantity fixed point represents badly — a 100-px edge gives 1e-4, which
/// is 6 in 16.16 — and a GPU has the divide for free, so dropping it from the
/// payload is both smaller and more accurate.
/// Copy this item's gradient stops into the frame buffer's third region, and record
/// where they landed in the item block.
///
/// The twin of `emit_edge_upload`, deliberately the same shape: one buffer, three
/// regions, one binding, with a per-item base index rather than a rebind. A gradient's
/// stops are variable-length per item for exactly the reason a polygon's edges are, so
/// the machinery that already solved that is the machinery to reuse.
///
/// The stops sit at the END of the geometry record — `slot1 − count * 5` — because
/// `__canvas_tailFor` appends them after whatever other tail the kind has. Computing
/// the base from the record's own length is what lets this work for a gradient-filled
/// polygon, whose tail is edges *then* stops, without the emitter knowing an edge count.
///
/// The offset is 16.16 like every other coordinate; the four channels are whole 0..255
/// bytes, which is how the item block already carries `fill` and `stroke`.
fn emit_gradient_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_grad_cursor: usize,
) {
    let head = builder.label("vk_grad_head");
    let done = builder.label("vk_grad_done");
    let empty = builder.label("vk_grad_empty");
    let copy = builder.label("vk_grad_copy");

    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let scale = abi::FP_SCRATCH[0];

    // Fewer than two stops is not a gradient, and the header already says so — the
    // count slot is written as 0 in that case, so one compare covers "no gradient",
    // "one stop" and "a kind with no interior".
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        off_header,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[6],
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(count, "2"));
    builder.emit(abi::branch_lt(&empty));

    // Does this frame still have room? The predicate already declined a scene whose
    // stops do not fit, so this is unreachable — kept for the reason the edge one is:
    // the alternative to declining is writing past the buffer, and truncating a ramp
    // would draw a *different picture*.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_grad_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], count));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_GRADIENT_STOPS.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&empty));

    builder.emit(abi::label(&copy));
    // The block carries the count and the base, in stops.
    builder.emit(abi::store_u32(
        count,
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_BASE,
    ));

    // target = mapped + (GRADIENT_BASE_WORDS + cursor * 5) * 4
    builder.emit(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &GRADIENT_STOP_WORDS.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        VULKAN_GRADIENT_BASE_WORDS,
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        2,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[1]));

    // The cursor moves on now, while the pre-advance value is already in the block.
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_grad_cursor,
    ));

    // source = header + (slot1 - count * 5) * 8, the record's own stop base.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], abi::SCRATCH[6], 8));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[8],
        count,
        abi::SCRATCH[7],
    ));
    builder.emit(abi::subtract_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        3,
    ));
    builder.emit(abi::add_registers(source, abi::SCRATCH[6], abi::SCRATCH[1]));

    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[0]));
    builder.emit(abi::move_immediate(index, "Integer", "0"));

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&done));
    // The offset, 16.16.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 0));
    builder.emit(abi::float_multiply_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
        scale,
    ));
    builder.emit(abi::float_round_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
    builder.emit(abi::add_immediate(target, target, 4));
    // The four channels, whole 0..255 — the form the block already uses for fill.
    for channel in 1..=4usize {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, channel * 8));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
    }
    builder.emit(abi::add_immediate(source, source, GRADIENT_STOP_WORDS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_BASE,
    ));

    builder.emit(abi::label(&done));
}

fn emit_edge_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_edge_cursor: usize,
    bands: &BandSlots,
) {
    let head = builder.label("vk_edge_head");
    let done = builder.label("vk_edge_done");
    let band = builder.label("vk_edge_band");
    let empty = builder.label("vk_edge_empty");
    let copy = builder.label("vk_edge_copy");
    let polyline = builder.label("vk_edge_polyline");
    let has_pieces = builder.label("vk_edge_pieces");
    let advanced = builder.label("vk_edge_advanced");
    let twice = builder.label("vk_edge_twice");
    let chords = builder.label("vk_edge_chords");
    let controls_init = builder.label("vk_edge_controls_init");
    let controls = builder.label("vk_edge_controls");

    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];

    // Only a polygon has edges. Every other kind leaves `misc.w` holding whatever
    // `HEADER_AUX0` meant for it — an arc's start angle — which is harmless because
    // the shader reads `misc.w` only in the polygon arm, but zeroing it keeps the
    // block meaning one thing at a time.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    let has_edges = builder.label("vk_edge_has");
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYGON));
    builder.emit(abi::branch_eq(&has_edges));
    // plan-154-C: a PolyLine's segments are edges in the polygon layout, one per segment
    // and without a wrap edge, so they take the same upload and band index -- the twin of
    // Metal's `emit_edge_buffer` (plan-154-B).
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYLINE));
    builder.emit(abi::branch_eq(&polyline));
    // plan-164-C: a curve shape's pieces take the same region, TWO slots a piece, and
    // the same band index -- the twin of Metal's `emit_edge_buffer`.
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYCURVE));
    builder.emit(abi::branch_eq(&has_pieces));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        GEO_KIND_CURVEPOLYGON,
    ));
    builder.emit(abi::branch_ne(&empty));
    builder.emit(abi::label(&has_pieces));
    builder.emit(abi::load_u32(
        count,
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], count));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[1], count));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&empty));
    builder.emit(abi::label(&polyline));
    // Its first and last live segments, as whole numbers over the 16.16 gradient axis
    // `emit_item_block` wrote: 16.16 holds a segment index only up to 32,767.
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_header,
    ));
    for (index, slot) in [(0usize, HEADER_POLYLINE_FIRST), (1, HEADER_POLYLINE_LAST)] {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[1],
            slot * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_item + ITEM_OFFSET_GRADIENT + index * 4,
        ));
    }
    builder.emit(abi::label(&has_edges));

    builder.emit(abi::load_u32(
        count,
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], count));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    // Unreachable: the pass walk (`__canvas_passNext`, bug-765) ends a pass before its
    // polygon edges pass the region, and a polygon past it alone sends the whole frame
    // to the software rasteriser.
    // Kept because the alternative to declining is writing past the buffer, and
    // truncating the polygon would draw a *different shape* — the same reason Metal's
    // `emit_edge_buffer` refuses rather than clamps.
    builder.emit(abi::branch(&empty));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    // target = mapped + cursor * 16
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        4,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[1]));
    // The cursor moves on now, while the pre-advance value is still the one written
    // into the item block above -- by one slot an edge, and by two a curve piece.
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::load_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], GEO_KIND_POLYCURVE));
    builder.emit(abi::branch_eq(&twice));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        GEO_KIND_CURVEPOLYGON,
    ));
    builder.emit(abi::branch_ne(&advanced));
    builder.emit(abi::label(&twice));
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::label(&advanced));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));

    // bug-687: each word's residual goes to the same edge's slot in the residual region,
    // a fixed distance past its 16.16 word (`VULKAN_EDGE_RESIDUAL_BASE_WORDS`) — Metal's
    // `emit_edge_buffer` twin.
    builder.emit(abi::move_immediate(
        residual_target,
        "Integer",
        &(VULKAN_EDGE_RESIDUAL_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(residual_target, residual_target, target));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[0],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[0]));
    builder.emit(abi::load_u64(source, abi::stack_pointer(), off_header));
    builder.emit(abi::add_immediate(source, source, HEADER_SLOTS * 8));
    builder.emit(abi::move_immediate(index, "Integer", "0"));

    // plan-164-C: a curve shape's pieces (`x0, y0, x1, y1, x2, y2, w1` each) go in as
    // two runs of `count` slots: each piece's chord `x0, y0, x2, y2` -- an edge to the
    // band index, a piece's two ends being its whole y-range -- then each piece's
    // control point and weight, `x1, y1, bits(w1), 0`. Metal's `emit_edge_buffer`, line for
    // line; the layout is at `CURVE_PIECE_SLOTS`.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYCURVE));
    builder.emit(abi::branch_eq(&chords));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        GEO_KIND_CURVEPOLYGON,
    ));
    builder.emit(abi::branch_ne(&head));

    // One double of the record as its 16.16 word and its residual, each cursor advanced.
    let word = |builder: &mut CodeBuilder, slot: usize| {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
        builder.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[0],
        ));
        builder.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
        builder.emit(abi::add_immediate(residual_target, residual_target, 4));
    };
    builder.emit(abi::label(&chords));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&controls_init));
    for slot in [0usize, 1, 4, 5] {
        word(builder, slot);
    }
    builder.emit(abi::add_immediate(source, source, CURVE_PIECE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&chords));
    builder.emit(abi::label(&controls_init));
    builder.emit(abi::load_u64(source, abi::stack_pointer(), off_header));
    builder.emit(abi::add_immediate(source, source, HEADER_SLOTS * 8));
    builder.emit(abi::move_immediate(index, "Integer", "0"));
    builder.emit(abi::label(&controls));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&band));
    for slot in [2usize, 3] {
        word(builder, slot);
    }
    // The weight as its fp32 BITS, not a 16.16 word: the shader's arithmetic on it is
    // single precision either way, and a weight has no 32767 to stay under.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 6 * 8));
    builder.emit(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 4));
    builder.emit(abi::add_immediate(target, target, 8));
    builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
    builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 4));
    builder.emit(abi::add_immediate(residual_target, residual_target, 8));
    builder.emit(abi::add_immediate(source, source, CURVE_PIECE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&controls));

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&band));
    // out[0..1] = (x0, y0); out[2..3] = (x0 + dx, y0 + dy), each as its 16.16 word and,
    // in the residual region, `round((v * 65536 - word) * 2^31)` (bug-687,
    // `EDGE_RESIDUAL_SCALE`). `v * 65536 - word` is exact: the two are within half a
    // step of each other.
    for (slot, delta) in [(0usize, None), (1, None), (0, Some(2usize)), (1, Some(3))] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
        if let Some(delta) = delta {
            builder.emit(abi::load_double(abi::FP_SCRATCH[2], source, delta * 8));
            builder.emit(abi::float_add_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[2],
            ));
        }
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
        builder.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[0],
        ));
        builder.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
        builder.emit(abi::add_immediate(residual_target, residual_target, 4));
    }
    builder.emit(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

    // bug-688: the polygon's edges are in the region; index them by band.
    builder.emit(abi::label(&band));
    emit_band_index(builder, bands, ITEM_BANDS);
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));

    builder.emit(abi::label(&done));
}

/// plan-163-B: the item's join records (`helper_joins.rs`) into the frame's edge region
/// after its own edges, two edge slots per record -- each record's four points as eight
/// 16.16 words, with their residuals at the residual region's same offsets, exactly as an
/// edge's endpoints travel. `strokeStyle.x` is the first slot and `strokeStyle.y` the
/// record count; both are 0 for an item with none, and so are the dash words. The
/// records sit right before the gradient stops, so they are found from the record's end:
/// `header + (slot1 - stops * 5 - records * 8) * 8`. Metal's `emit_join_buffer` twin.
/// plan-163-C: the clip's item-block words -- `clipPath` = (first clip edge, clip edge
/// count, mode | rule << 2, a picture's transparency step T) and `clipBand`'s band count
/// 0 (the shader walks every clip edge) -- and a path clip's edges, copied from the
/// record's tail (`slot1 − stops·5 − edges·5`, right before the stops) into the frame's
/// edge region after the item's own edges and join records, as 16.16 words with their
/// residuals, exactly as a polygon's edges are (`emit_edge_upload`). The frame-sum edge cap
/// counts them, so the over-cap arm is unreachable; it leaves the edge count 0, which the
/// shader reads as a clip enclosing nothing -- hidden, never drawn unclipped.
fn emit_clip_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_edge_cursor: usize,
    bands: &BandSlots,
) {
    let head = builder.label("vk_clip_head");
    let done = builder.label("vk_clip_done");
    let copy = builder.label("vk_clip_copy");
    let banded = builder.label("vk_clip_banded");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[6];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];
    let clip = off_item + ITEM_OFFSET_CLIP_PATH;
    let band = off_item + ITEM_OFFSET_CLIP_BAND;

    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    for word in [0usize, 4, 8, 12] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            band + word,
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            clip + word,
        ));
    }
    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    // mode | rule << 2, and T.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_MODE * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[5],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_RULE * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        2,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[5],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_MODE,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_TRANSPARENCY * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_PICTURE_T,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[5], "2"));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(count, "0"));
    builder.emit(abi::branch_le(&done));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[7], abi::SCRATCH[5], count));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[8],
        "Integer",
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[7], abi::SCRATCH[8]));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_BASE,
    ));
    builder.emit(abi::store_u32(
        count,
        abi::stack_pointer(),
        clip + ITEM_CLIP_PATH_COUNT,
    ));
    // target = mapped + base * 16; the residuals a fixed distance past it.
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[7],
        abi::SCRATCH[5],
        4,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[7]));
    builder.emit(abi::move_immediate(
        residual_target,
        "Integer",
        &(VULKAN_EDGE_RESIDUAL_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(residual_target, residual_target, target));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[5],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[5]));
    // source = header + (slot1 - stops * 5 - count * 5) * 8.
    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, 8));
    builder.emit(abi::float_convert_to_signed_x(source, abi::FP_SCRATCH[1]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[7], abi::SCRATCH[7], count));
    builder.emit(abi::move_immediate(abi::SCRATCH[8], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    builder.emit(abi::shift_left_immediate(source, source, 3));
    builder.emit(abi::add_registers(source, header, source));
    builder.emit(abi::move_immediate(index, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&banded));
    for (slot, delta) in [(0usize, None), (1, None), (0, Some(2usize)), (1, Some(3))] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, slot * 8));
        if let Some(delta) = delta {
            builder.emit(abi::load_double(abi::FP_SCRATCH[2], source, delta * 8));
            builder.emit(abi::float_add_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[2],
            ));
        }
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[5], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
        builder.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[5],
        ));
        builder.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[5],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[5], residual_target, 0));
        builder.emit(abi::add_immediate(residual_target, residual_target, 4));
    }
    builder.emit(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));
    // The clip's edges are in the region; index them by band.
    builder.emit(abi::label(&banded));
    emit_band_index(builder, bands, CLIP_BANDS);
    builder.emit(abi::label(&done));
}

/// plan-163-B: copy this item's dash words into the frame buffer's dash region, and name
/// the slice in its item block — `strokeStyle.z` the first word, `strokeStyle.w` the word
/// count (`emit_join_upload` zeroed both, which the shader reads as a solid stroke).
///
/// The words are float32 bit patterns: the phase (header slot `HEADER_DASH_PHASE`), then
/// the record's dash tail — the entry count, the period, each entry's end, and the kind's
/// perimeter data (`helper_dashes.rs`) — found at `slot1 − stops·5 − dashes`, right before
/// the gradient stops. Floats rather than 16.16: the pattern's lengths and an outline's
/// perimeter positions are compared, never added to a coordinate, and fp32 holds a
/// thousand-pixel perimeter to 6e-5 px. A frame whose words would pass the region's cap
/// is declined by the predicate, so the over-cap arm is unreachable and leaves the stroke
/// solid rather than write past the region.
fn emit_dash_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_dash_cursor: usize,
) {
    let head = builder.label("vk_dash_head");
    let done = builder.label("vk_dash_done");
    let copy = builder.label("vk_dash_copy");
    let header = abi::SCRATCH[0];
    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let cursor = abi::SCRATCH[5];
    let target = abi::SCRATCH[6];
    let style = off_item + ITEM_OFFSET_STROKE_STYLE;

    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_DASH_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(count, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(count, "0"));
    builder.emit(abi::branch_le(&done));
    builder.emit(abi::load_u64(cursor, abi::stack_pointer(), off_dash_cursor));
    builder.emit(abi::add_registers(abi::SCRATCH[1], cursor, count));
    builder.emit(abi::add_immediate(abi::SCRATCH[1], abi::SCRATCH[1], 1));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[7],
        "Integer",
        &VULKAN_MAX_FRAME_DASH_WORDS.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[7]));
    builder.emit(abi::branch_le(&copy));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_dash_cursor,
    ));
    builder.emit(abi::store_u32(
        cursor,
        abi::stack_pointer(),
        style + ITEM_STROKE_DASH_BASE,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[7], count, 1));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        style + ITEM_STROKE_DASH_COUNT,
    ));
    // target = mapped + (VULKAN_DASH_BASE_WORDS + cursor) * 4.
    builder.emit(abi::add_immediate(
        abi::SCRATCH[1],
        cursor,
        VULKAN_DASH_BASE_WORDS,
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[1],
        2,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[1]));
    // The header again: the state load may have used the scratch bank.
    builder.emit(abi::load_u64(header, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_DASH_PHASE * 8,
    ));
    builder.emit(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[7], target, 0));
    builder.emit(abi::add_immediate(target, target, 4));
    // source = header + (slot1 - stops * 5 - count) * 8.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, 8));
    builder.emit(abi::float_convert_to_signed_x(source, abi::FP_SCRATCH[1]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[8], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    // plan-163-C: the path clip's edges sit between the dashes and the stops.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[8],
    ));
    builder.emit(abi::subtract_registers(source, source, abi::SCRATCH[7]));
    builder.emit(abi::subtract_registers(source, source, count));
    builder.emit(abi::shift_left_immediate(source, source, 3));
    builder.emit(abi::add_registers(source, header, source));
    builder.emit(abi::move_immediate(index, "Integer", "0"));
    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 0));
    builder.emit(abi::float_convert_s_from_d(
        abi::FP_SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::float_move_x_from_d(
        abi::SCRATCH[7],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[7], target, 0));
    builder.emit(abi::add_immediate(target, target, 4));
    builder.emit(abi::add_immediate(source, source, 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
}

fn emit_join_upload(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_header: usize,
    off_edge_cursor: usize,
) {
    let head = builder.label("vk_join_head");
    let done = builder.label("vk_join_done");
    let none = builder.label("vk_join_none");
    let copy = builder.label("vk_join_copy");
    let records = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let slots = abi::SCRATCH[6];
    let residual_target = abi::SCRATCH[1];
    let scale = abi::FP_SCRATCH[0];
    let residual_scale = abi::FP_SCRATCH[3];
    let style = off_item + ITEM_OFFSET_STROKE_STYLE;

    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in [ITEM_STROKE_DASH_BASE, ITEM_STROKE_DASH_COUNT] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            style + word,
        ));
    }
    builder.emit(abi::load_u64(source, abi::stack_pointer(), off_header));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_JOIN_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(records, abi::FP_SCRATCH[1]));
    builder.emit(abi::compare_immediate(records, "0"));
    builder.emit(abi::branch_le(&none));
    builder.emit(abi::shift_left_immediate(slots, records, 1));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::add_registers(abi::SCRATCH[1], abi::SCRATCH[0], slots));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[1],
        &VULKAN_MAX_FRAME_EDGES.to_string(),
    ));
    builder.emit(abi::branch_le(&copy));
    // Unreachable: `__canvas_blockCost` counts two edges per record, and the pass walk
    // ends a pass before its total passes the region (bug-765). Kept because the
    // alternative is a write past the buffer; dropping the records draws the corners
    // round, not a wrong shape.
    builder.emit(abi::branch(&none));

    builder.emit(abi::label(&copy));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        style + ITEM_STROKE_JOIN_BASE,
    ));
    builder.emit(abi::store_u32(
        records,
        abi::stack_pointer(),
        style + ITEM_STROKE_JOIN_COUNT,
    ));
    // target = mapped + cursor * 16; the cursor moves past the records' slots.
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        4,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[1]));
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], slots));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
    ));
    builder.emit(abi::move_immediate(
        residual_target,
        "Integer",
        &(VULKAN_EDGE_RESIDUAL_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(residual_target, residual_target, target));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        EDGE_RESIDUAL_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        residual_scale,
        abi::SCRATCH[0],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(scale, abi::SCRATCH[0]));
    // source = header + (slot1 - stops * 5 - dashes - records * 8) * 8: the dash tail
    // (plan-163-B) sits between the records and the stops.
    builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, 8));
    builder.emit(abi::float_convert_to_signed_x(index, abi::FP_SCRATCH[1]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_GRADIENT_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[7], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    builder.emit(abi::shift_left_immediate(abi::SCRATCH[0], records, 3));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_DASH_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    // plan-163-C: and the path clip's edges, five floats each, after the dashes.
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        source,
        HEADER_CLIP_EDGE_COUNT * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[0],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[7], "Integer", "5"));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[7],
    ));
    builder.emit(abi::subtract_registers(index, index, abi::SCRATCH[0]));
    builder.emit(abi::shift_left_immediate(index, index, 3));
    builder.emit(abi::add_registers(source, source, index));
    builder.emit(abi::move_immediate(index, "Integer", "0"));

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, slots));
    builder.emit(abi::branch_ge(&done));
    for word in 0..4usize {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], source, word * 8));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
        builder.emit(abi::add_immediate(target, target, 4));
        builder.emit(abi::signed_convert_to_float_d(
            abi::FP_SCRATCH[2],
            abi::SCRATCH[0],
        ));
        builder.emit(abi::float_subtract_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[2],
        ));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            residual_scale,
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[0],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(abi::SCRATCH[0], residual_target, 0));
        builder.emit(abi::add_immediate(residual_target, residual_target, 4));
    }
    builder.emit(abi::add_immediate(source, source, 32));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&none));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in [ITEM_STROKE_JOIN_BASE, ITEM_STROKE_JOIN_COUNT] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            style + word,
        ));
    }
    builder.emit(abi::label(&done));
}

/// The stack slots `emit_band_index` works in (bug-688): the frame's band-region cursor
/// and one polygon's working state — its first and past-the-last 16.16 edge in the
/// shared buffer, its lowest edge y (16.16), the band reach `2r` (16.16), the band
/// height (16.16), the band count, and the table's address.
struct BandSlots {
    state: usize,
    item: usize,
    header: usize,
    cursor: usize,
    edges: usize,
    end: usize,
    ymin: usize,
    reach: usize,
    height: usize,
    count: usize,
    table: usize,
}

impl BandSlots {
    fn allocate(builder: &mut CodeBuilder, state: usize, item: usize, header: usize) -> Self {
        let mut slot = |name: &str| builder.allocate_stack_object(name, 8);
        BandSlots {
            state,
            item,
            header,
            cursor: slot("vk_band_cursor"),
            edges: slot("vk_band_edges"),
            end: slot("vk_band_end"),
            ymin: slot("vk_band_ymin"),
            reach: slot("vk_band_reach"),
            height: slot("vk_band_height"),
            count: slot("vk_band_count"),
            table: slot("vk_band_table"),
        }
    }
}

/// Which edge set a band index is built for: the item's own edges, its header in
/// `arcCaps`, or (plan-163-C) a path clip's, its header in `clipBand` -- Metal's
/// `BandSet`, with offsets into the block rather than into Metal's frame.
#[derive(Clone, Copy)]
struct BandSet {
    count: usize,
    base: usize,
    header: usize,
    clip: bool,
}

const ITEM_BANDS: BandSet = BandSet {
    count: ITEM_OFFSET_MISC + 12,
    base: ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    header: ITEM_OFFSET_ARC_CAPS,
    clip: false,
};

const CLIP_BANDS: BandSet = BandSet {
    count: ITEM_OFFSET_CLIP_PATH + ITEM_CLIP_PATH_COUNT,
    base: ITEM_OFFSET_CLIP_PATH + ITEM_CLIP_PATH_BASE,
    header: ITEM_OFFSET_CLIP_BAND,
    clip: true,
};

/// Build this polygon's band index in the shared buffer's band region and write its
/// header into the item block's `arcCaps` (bug-688; layout at `VULKAN_BAND_BASE_WORDS`)
/// -- or a path clip's into `clipBand` (`set`).
///
/// The Vulkan port of Metal's `emit_band_index` (bug-686 section G), the same algorithm
/// step for step, so a polygon's bands — and therefore its pixels — are the same on both
/// backends. It is a port rather than one shared builder because Metal's is written
/// against the macOS `Asm` and a hand-numbered stack frame, and sharing would re-emit
/// Metal's instructions; bug-688 keeps Metal unchanged.
///
/// Runs after `emit_edge_upload` has converted the polygon's edges, and works on those
/// 16.16 words — exactly what the shader reads — rather than on the header's doubles,
/// so the band arithmetic is integer and a rounding difference between the two cannot
/// put an edge in the wrong band.
///
/// ## Why the result is bit-identical to the full loop
///
/// An edge is put in every band its `[ylo − r, yhi + r]` meets, so a band holds (a)
/// every edge whose y-span contains any row of the band — the only edges the even-odd
/// crossing test counts, so `inside` is exact — and (b) every edge within `r` of any
/// point of the band — so `best` is exact whenever the true distance is at most `r`,
/// and otherwise both loops' `best` exceed `r`, where coverage has clamped:
///
/// * **Untransformed**, a fill clamps once `|d| ≥ 0.5` and a stroke once
///   `|d| − half ≥ 0.5`, so `r = half + 0.5`, with `half = max(strokeHalf, 0)`.
/// * **Transformed**, the shader evaluates five shape-space points against the centre's
///   band; with `F = ‖M‖_F` (M the inverse matrix) `r = half + 0.5 + 2F` covers every
///   neighbour and every scale the shader can divide by — the argument is written out at
///   Metal's `emit_band_index`.
///
/// Plus a **1-unit guard** in shape space, for the float error of the shader's band
/// lookup against these integer bands.
///
/// ## Sizing
///
/// Bands are 2 px tall untransformed, `√2·F` transformed, and taller when the span needs
/// more than `min(edges, BAND_MAX)` bands. If the list would pass `BAND_LIST_PER_EDGE`
/// entries per edge the height doubles until it fits; at one band the list is every edge
/// once. So one polygon never needs more than `BAND_WORDS_PER_EDGE` words per edge, which
/// the region is sized for; the region-full branch is unreachable and kept because the
/// alternative is a write past the buffer. It, a polygon under `BAND_MIN_EDGES`, and a
/// transform whose matrix is not finite write band count 0: the shader loops over every
/// edge. Never a decline, never a truncation.
///
/// ## Differences from the AArch64-only original
///
/// * **Every float-to-integer result is checked for a negative value.** AArch64's
///   `fcvtps` saturates an infinity to `i64::MAX`, which the `2^30` bound catches; x86-64's
///   `cvtsd2si` returns `i64::MIN` for anything out of range, which that bound does not.
/// * No calls; `SCRATCH[0..=8]` only, and no `compare_immediate` against a constant wider
///   than 12 bits (on AArch64 its encoding borrows x16/x17, which are `SCRATCH[7]`/`[8]`).
fn emit_band_index(builder: &mut CodeBuilder, at: &BandSlots, set: BandSet) {
    let none = builder.label("vk_band_none");
    let done = builder.label("vk_band_done");
    let s = abi::SCRATCH;
    let f = abi::FP_SCRATCH;
    let block = |field: usize| at.item + set.header + field;
    let sp = abi::stack_pointer;

    // --- which polygons: E >= BAND_MIN_EDGES --------------------------------------
    builder.emit(abi::load_u32(s[2], sp(), at.item + set.count));
    builder.emit(abi::move_immediate(
        s[3],
        "Integer",
        &BAND_MIN_EDGES.to_string(),
    ));
    builder.emit(abi::compare_registers(s[2], s[3]));
    builder.emit(abi::branch_lt(&none));

    // --- the edges: P = mapped + base*16, END = P + E*16 ------------------------------
    builder.emit(abi::load_u32(s[0], sp(), at.item + set.base));
    builder.emit(abi::shift_left_immediate(s[0], s[0], 4));
    emit_state_load(builder, at.state, GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED, s[1]);
    builder.emit(abi::add_registers(s[0], s[0], s[1]));
    builder.emit(abi::store_u64(s[0], sp(), at.edges));
    builder.emit(abi::shift_left_immediate(s[1], s[2], 4));
    builder.emit(abi::add_registers(s[1], s[0], s[1]));
    builder.emit(abi::store_u64(s[1], sp(), at.end));

    // --- ymin (s2) and ymax (s3) over both endpoints of every edge ----------------
    let measure_head = builder.label("vk_band_measure_head");
    let measure_done = builder.label("vk_band_measure_done");
    builder.emit(abi::load_u32(s[2], s[0], 4));
    builder.emit(abi::sign_extend_word(s[2], s[2]));
    builder.emit(abi::move_register(s[3], s[2]));
    builder.emit(abi::label(&measure_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&measure_done));
    for offset in [4usize, 12] {
        let not_lower = builder.label("vk_band_not_lower");
        let not_higher = builder.label("vk_band_not_higher");
        builder.emit(abi::load_u32(s[4], s[0], offset));
        builder.emit(abi::sign_extend_word(s[4], s[4]));
        builder.emit(abi::compare_registers(s[4], s[2]));
        builder.emit(abi::branch_ge(&not_lower));
        builder.emit(abi::move_register(s[2], s[4]));
        builder.emit(abi::label(&not_lower));
        builder.emit(abi::compare_registers(s[4], s[3]));
        builder.emit(abi::branch_le(&not_higher));
        builder.emit(abi::move_register(s[3], s[4]));
        builder.emit(abi::label(&not_higher));
    }
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::branch(&measure_head));
    builder.emit(abi::label(&measure_done));
    builder.emit(abi::store_u64(s[2], sp(), at.ymin));
    // s3 = the edges' own y extent; s2 stays ymin through the reach computation.
    builder.emit(abi::subtract_registers(s[3], s[3], s[2]));

    // --- the reach r (f1) and the minimum band height (f4), in shape units --------
    builder.emit(abi::load_u64(s[0], sp(), at.header));
    builder.emit(abi::load_double(f[1], s[0], HEADER_STROKE_HALF * 8));
    // plan-163-C: a clip's coverage clamps at `|d| >= 0.5` -- no stroke reaches past it --
    // so its half-width is 0 (and its own cap and join reach do not apply).
    let stroke_reach_done = builder.label("vk_band_stroke_reach_done");
    if set.clip {
        builder.emit(abi::move_immediate(s[4], "Integer", "0"));
        builder.emit(abi::signed_convert_to_float_d(f[1], s[4]));
        builder.emit(abi::branch(&stroke_reach_done));
    }
    // plan-154-C: a PolyLine keeps its half-width in the RADIUS slot (it is
    // stroke-as-fill, slot 7 is -1); a polygon never writes that slot, so its reach is
    // unchanged. Metal's `emit_band_index` takes the same maximum.
    builder.emit(abi::load_double(f[2], s[0], HEADER_RADIUS * 8));
    builder.emit(abi::float_max_d(f[1], f[1], f[2]));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::signed_convert_to_float_d(f[2], s[4]));
    // `fmaxnm`: a NaN half-width reads as 0 rather than poisoning the reach.
    builder.emit(abi::float_max_d(f[1], f[1], f[2]));
    // plan-163-B: a Square cap reaches `half * sqrt(2)` past its endpoint.
    let square_reach = builder.label("vk_band_square_reach");
    for instruction in emit_square_cap_reach(s[0], f[1], s[4], f[2], &square_reach) {
        builder.emit(instruction);
    }
    // plan-163-B: and to a join record's reach (0 for an item with none), so a miter's
    // point in a row's band is reached through the edge it sits at the start of.
    builder.emit(abi::load_double(f[2], s[0], HEADER_JOIN_REACH * 8));
    builder.emit(abi::float_max_d(f[1], f[1], f[2]));
    builder.emit(abi::label(&stroke_reach_done));
    builder.emit(abi::move_immediate(s[4], "Integer", "3"));
    builder.emit(abi::signed_convert_to_float_d(f[2], s[4]));
    builder.emit(abi::move_immediate(s[4], "Integer", "2"));
    builder.emit(abi::signed_convert_to_float_d(f[3], s[4]));
    builder.emit(abi::float_divide_d(f[2], f[2], f[3]));
    // r = half + 0.5 (the clamp) + 1 (the guard)
    builder.emit(abi::float_add_d(f[1], f[1], f[2]));
    builder.emit(abi::float_move_d_from_d(f[4], f[3]));
    let untransformed = builder.label("vk_band_untransformed");
    builder.emit(abi::load_double(f[5], s[0], HEADER_HAS_TRANSFORM * 8));
    builder.emit(abi::float_convert_to_signed_x(s[4], f[5]));
    builder.emit(abi::compare_immediate(s[4], "0"));
    builder.emit(abi::branch_eq(&untransformed));
    // F = ||M||_F from the four matrix terms, which the header carries as float32 BIT
    // PATTERNS (`__canvas_float32Bits`). The assemblers have no single->double convert,
    // so the bits are rebuilt as a double by hand: exponent rebiased by 1023 - 127 =
    // 896, mantissa shifted up 52 - 23 = 29. The sign is dropped (the term is squared);
    // a zero or subnormal term counts as 0, which the 1-unit guard dwarfs; an infinite
    // or NaN one gives the polygon no bands.
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::signed_convert_to_float_d(f[6], s[4]));
    for slot in [
        HEADER_TRANSFORM_IA,
        HEADER_TRANSFORM_IB,
        HEADER_TRANSFORM_IC,
        HEADER_TRANSFORM_ID,
    ] {
        let zero_term = builder.label("vk_band_zero_term");
        builder.emit(abi::load_double(f[7], s[0], slot * 8));
        builder.emit(abi::float_convert_to_signed_x(s[4], f[7]));
        builder.emit(abi::shift_right_immediate(s[5], s[4], 23));
        builder.emit(abi::move_immediate(s[6], "Integer", "255"));
        builder.emit(abi::and_registers(s[5], s[5], s[6]));
        builder.emit(abi::compare_registers(s[5], s[6]));
        builder.emit(abi::branch_eq(&none));
        builder.emit(abi::compare_immediate(s[5], "0"));
        builder.emit(abi::branch_eq(&zero_term));
        builder.emit(abi::add_immediate(s[5], s[5], 896));
        builder.emit(abi::shift_left_immediate(s[5], s[5], 52));
        builder.emit(abi::move_immediate(s[6], "Integer", "8388607"));
        builder.emit(abi::and_registers(s[7], s[4], s[6]));
        builder.emit(abi::shift_left_immediate(s[7], s[7], 29));
        builder.emit(abi::or_registers(s[5], s[5], s[7]));
        builder.emit(abi::float_move_d_from_x(f[7], s[5]));
        builder.emit(abi::float_multiply_d(f[7], f[7], f[7]));
        builder.emit(abi::float_add_d(f[6], f[6], f[7]));
        builder.emit(abi::label(&zero_term));
    }
    builder.emit(abi::float_sqrt_d(f[6], f[6]));
    // r += 2F; the minimum height is sqrt(2)*F, about two surface pixels.
    builder.emit(abi::float_add_d(f[7], f[6], f[6]));
    builder.emit(abi::float_add_d(f[1], f[1], f[7]));
    builder.emit(abi::float_sqrt_d(f[7], f[3]));
    builder.emit(abi::float_multiply_d(f[4], f[6], f[7]));
    builder.emit(abi::label(&untransformed));

    // --- to 16.16, rounding UP: s4 = r, s5 = the minimum height ---------------------
    builder.emit(abi::move_immediate(s[4], "Integer", FIXED_POINT_SCALE));
    builder.emit(abi::signed_convert_to_float_d(f[7], s[4]));
    builder.emit(abi::float_multiply_d(f[1], f[1], f[7]));
    builder.emit(abi::float_ceil_to_signed_x(s[4], f[1]));
    builder.emit(abi::float_multiply_d(f[4], f[4], f[7]));
    builder.emit(abi::float_ceil_to_signed_x(s[5], f[4]));
    // A reach or height past 2^30 (16,384 units) is a degenerate transform. AArch64
    // saturates an out-of-range convert to i64::MAX, which the bound catches; x86-64
    // answers i64::MIN, which the sign test catches.
    builder.emit(abi::compare_immediate(s[4], "0"));
    builder.emit(abi::branch_lt(&none));
    builder.emit(abi::compare_immediate(s[5], "0"));
    builder.emit(abi::branch_lt(&none));
    builder.emit(abi::move_immediate(
        s[6],
        "Integer",
        &(1u64 << 30).to_string(),
    ));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_gt(&none));
    builder.emit(abi::compare_registers(s[5], s[6]));
    builder.emit(abi::branch_gt(&none));
    let height_positive = builder.label("vk_band_height_positive");
    builder.emit(abi::compare_immediate(s[5], "1"));
    builder.emit(abi::branch_ge(&height_positive));
    builder.emit(abi::move_immediate(s[5], "Integer", "1"));
    builder.emit(abi::label(&height_positive));

    // --- span = extent + 2r (s3); top = ymin - r must be an int32, and so must the
    // bottom, because the shader reads `top` as a 16.16 `int` ----------------------
    builder.emit(abi::shift_left_immediate(s[4], s[4], 1));
    builder.emit(abi::store_u64(s[4], sp(), at.reach));
    builder.emit(abi::add_registers(s[3], s[3], s[4]));
    builder.emit(abi::shift_right_immediate(s[6], s[4], 1));
    builder.emit(abi::subtract_registers(s[7], s[2], s[6]));
    // The encoder parses an immediate as u64, so i32::MIN goes in as its 64-bit two's
    // complement.
    builder.emit(abi::move_immediate(
        s[8],
        "Integer",
        &(i64::from(i32::MIN) as u64).to_string(),
    ));
    builder.emit(abi::compare_registers(s[7], s[8]));
    builder.emit(abi::branch_lt(&none));
    builder.emit(abi::add_registers(s[8], s[7], s[3]));
    builder.emit(abi::move_immediate(s[6], "Integer", &i32::MAX.to_string()));
    builder.emit(abi::compare_registers(s[8], s[6]));
    builder.emit(abi::branch_gt(&none));

    // --- height = max(minimum, ceil(span / (min(E, BAND_MAX) - 1))) -----------------
    // At most `min(E, BAND_MAX)` bands: span / height <= that - 1.
    let bands_capped = builder.label("vk_band_bands_capped");
    builder.emit(abi::load_u32(s[6], sp(), at.item + set.count));
    builder.emit(abi::move_immediate(s[7], "Integer", &BAND_MAX.to_string()));
    builder.emit(abi::compare_registers(s[6], s[7]));
    builder.emit(abi::branch_le(&bands_capped));
    builder.emit(abi::move_register(s[6], s[7]));
    builder.emit(abi::label(&bands_capped));
    const _: () = assert!(BAND_MIN_EDGES >= 2 && BAND_MAX >= 2);
    builder.emit(abi::subtract_immediate(s[6], s[6], 1));
    builder.emit(abi::add_registers(s[7], s[3], s[6]));
    builder.emit(abi::subtract_immediate(s[7], s[7], 1));
    builder.emit(abi::unsigned_divide_registers(s[7], s[7], s[6]));
    let height_chosen = builder.label("vk_band_height_chosen");
    builder.emit(abi::compare_registers(s[7], s[5]));
    builder.emit(abi::branch_ge(&height_chosen));
    builder.emit(abi::move_register(s[7], s[5]));
    builder.emit(abi::label(&height_chosen));
    builder.emit(abi::store_u64(s[7], sp(), at.height));

    // --- size the list; double the height until it is within its budget -----------
    // s3 (span) is live through this loop.
    let fit_head = builder.label("vk_band_fit_head");
    let size_head = builder.label("vk_band_size_head");
    let size_done = builder.label("vk_band_size_done");
    let fits = builder.label("vk_band_fits");
    builder.emit(abi::label(&fit_head));
    builder.emit(abi::load_u64(s[4], sp(), at.height));
    builder.emit(abi::unsigned_divide_registers(s[6], s[3], s[4]));
    builder.emit(abi::add_immediate(s[6], s[6], 1));
    builder.emit(abi::store_u64(s[6], sp(), at.count));
    builder.emit(abi::move_immediate(s[2], "Integer", "0"));
    builder.emit(abi::load_u64(s[0], sp(), at.edges));
    builder.emit(abi::load_u64(s[1], sp(), at.end));
    builder.emit(abi::label(&size_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&size_done));
    emit_band_range(builder, at);
    builder.emit(abi::subtract_registers(s[5], s[5], s[4]));
    builder.emit(abi::add_immediate(s[5], s[5], 1));
    builder.emit(abi::add_registers(s[2], s[2], s[5]));
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::branch(&size_head));
    builder.emit(abi::label(&size_done));
    builder.emit(abi::load_u32(s[4], sp(), at.item + set.count));
    builder.emit(abi::move_immediate(
        s[5],
        "Integer",
        &BAND_LIST_PER_EDGE.to_string(),
    ));
    builder.emit(abi::multiply_registers(s[4], s[4], s[5]));
    builder.emit(abi::compare_registers(s[2], s[4]));
    builder.emit(abi::branch_le(&fits));
    builder.emit(abi::load_u64(s[4], sp(), at.height));
    builder.emit(abi::shift_left_immediate(s[4], s[4], 1));
    builder.emit(abi::store_u64(s[4], sp(), at.height));
    builder.emit(abi::branch(&fit_head));
    builder.emit(abi::label(&fits));

    // --- claim 2*bands + list words of the region ---------------------------------
    builder.emit(abi::load_u64(s[6], sp(), at.count));
    builder.emit(abi::shift_left_immediate(s[4], s[6], 1));
    builder.emit(abi::add_registers(s[4], s[4], s[2]));
    builder.emit(abi::load_u64(s[5], sp(), at.cursor));
    builder.emit(abi::add_registers(s[4], s[4], s[5]));
    builder.emit(abi::move_immediate(
        s[7],
        "Integer",
        &VULKAN_MAX_FRAME_BAND_WORDS.to_string(),
    ));
    builder.emit(abi::compare_registers(s[4], s[7]));
    builder.emit(abi::branch_gt(&none));
    builder.emit(abi::store_u64(s[4], sp(), at.cursor));

    // --- the header: top, height, count, table offset -----------------------------
    builder.emit(abi::store_u32(s[5], sp(), block(ITEM_BAND_START)));
    builder.emit(abi::store_u32(s[6], sp(), block(ITEM_BAND_COUNT)));
    builder.emit(abi::load_u64(s[4], sp(), at.height));
    builder.emit(abi::store_u32(s[4], sp(), block(ITEM_BAND_HEIGHT)));
    builder.emit(abi::load_u64(s[4], sp(), at.reach));
    builder.emit(abi::shift_right_immediate(s[4], s[4], 1));
    builder.emit(abi::load_u64(s[7], sp(), at.ymin));
    builder.emit(abi::subtract_registers(s[7], s[7], s[4]));
    builder.emit(abi::store_u32(s[7], sp(), block(ITEM_BAND_TOP)));

    // --- the table's address: mapped + BAND_BASE*4 + start*4 ----------------------
    builder.emit(abi::shift_left_immediate(s[5], s[5], 2));
    emit_state_load(builder, at.state, GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED, s[4]);
    builder.emit(abi::add_registers(s[5], s[5], s[4]));
    builder.emit(abi::move_immediate(
        s[4],
        "Integer",
        &(VULKAN_BAND_BASE_WORDS * 4).to_string(),
    ));
    builder.emit(abi::add_registers(s[5], s[5], s[4]));
    builder.emit(abi::store_u64(s[5], sp(), at.table));

    // --- counts to zero: table[2k + 1] = 0 (s5 table, s6 bands) --------------------
    let zero_head = builder.label("vk_band_zero_head");
    let zero_done = builder.label("vk_band_zero_done");
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::move_immediate(s[7], "Integer", "0"));
    builder.emit(abi::label(&zero_head));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_ge(&zero_done));
    builder.emit(abi::shift_left_immediate(s[8], s[4], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[5]));
    builder.emit(abi::store_u32(s[7], s[8], 4));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&zero_head));
    builder.emit(abi::label(&zero_done));

    // --- count each band's edges -----------------------------------------------------
    let count_head = builder.label("vk_band_count_head");
    let count_band = builder.label("vk_band_count_band");
    let count_next = builder.label("vk_band_count_next");
    let count_done = builder.label("vk_band_count_done");
    builder.emit(abi::load_u64(s[0], sp(), at.edges));
    builder.emit(abi::load_u64(s[1], sp(), at.end));
    builder.emit(abi::load_u64(s[2], sp(), at.table));
    builder.emit(abi::label(&count_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&count_done));
    emit_band_range(builder, at);
    builder.emit(abi::label(&count_band));
    builder.emit(abi::compare_registers(s[4], s[5]));
    builder.emit(abi::branch_gt(&count_next));
    builder.emit(abi::shift_left_immediate(s[6], s[4], 3));
    builder.emit(abi::add_registers(s[6], s[6], s[2]));
    builder.emit(abi::load_u32(s[7], s[6], 4));
    builder.emit(abi::add_immediate(s[7], s[7], 1));
    builder.emit(abi::store_u32(s[7], s[6], 4));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&count_band));
    builder.emit(abi::label(&count_next));
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::branch(&count_head));
    builder.emit(abi::label(&count_done));

    // --- each band's first list entry: table[2k] = sum of the counts before it -----
    let start_head = builder.label("vk_band_start_head");
    let start_done = builder.label("vk_band_start_done");
    builder.emit(abi::load_u64(s[6], sp(), at.count));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::move_immediate(s[7], "Integer", "0"));
    builder.emit(abi::label(&start_head));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_ge(&start_done));
    builder.emit(abi::shift_left_immediate(s[8], s[4], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[2]));
    builder.emit(abi::store_u32(s[7], s[8], 0));
    builder.emit(abi::load_u32(s[5], s[8], 4));
    builder.emit(abi::add_registers(s[7], s[7], s[5]));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&start_head));
    builder.emit(abi::label(&start_done));

    // --- the list: each edge's number into each of its bands, using table[2k] as the
    // band's fill cursor (s8 = the list, s3 = the edge number) ------------------------
    let fill_head = builder.label("vk_band_fill_head");
    let fill_band = builder.label("vk_band_fill_band");
    let fill_next = builder.label("vk_band_fill_next");
    let fill_done = builder.label("vk_band_fill_done");
    builder.emit(abi::shift_left_immediate(s[8], s[6], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[2]));
    builder.emit(abi::load_u64(s[0], sp(), at.edges));
    builder.emit(abi::load_u64(s[1], sp(), at.end));
    builder.emit(abi::move_immediate(s[3], "Integer", "0"));
    builder.emit(abi::label(&fill_head));
    builder.emit(abi::compare_registers(s[0], s[1]));
    builder.emit(abi::branch_ge(&fill_done));
    emit_band_range(builder, at);
    builder.emit(abi::label(&fill_band));
    builder.emit(abi::compare_registers(s[4], s[5]));
    builder.emit(abi::branch_gt(&fill_next));
    builder.emit(abi::shift_left_immediate(s[6], s[4], 3));
    builder.emit(abi::add_registers(s[6], s[6], s[2]));
    builder.emit(abi::load_u32(s[7], s[6], 0));
    builder.emit(abi::add_immediate(s[7], s[7], 1));
    builder.emit(abi::store_u32(s[7], s[6], 0));
    builder.emit(abi::subtract_immediate(s[7], s[7], 1));
    builder.emit(abi::shift_left_immediate(s[7], s[7], 2));
    builder.emit(abi::add_registers(s[7], s[7], s[8]));
    builder.emit(abi::store_u32(s[3], s[7], 0));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&fill_band));
    builder.emit(abi::label(&fill_next));
    builder.emit(abi::add_immediate(s[0], s[0], 16));
    builder.emit(abi::add_immediate(s[3], s[3], 1));
    builder.emit(abi::branch(&fill_head));
    builder.emit(abi::label(&fill_done));

    // --- the fill cursors ran to each band's end; take the counts back off ---------
    let rewind_head = builder.label("vk_band_rewind_head");
    builder.emit(abi::load_u64(s[6], sp(), at.count));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    builder.emit(abi::label(&rewind_head));
    builder.emit(abi::compare_registers(s[4], s[6]));
    builder.emit(abi::branch_ge(&done));
    builder.emit(abi::shift_left_immediate(s[8], s[4], 3));
    builder.emit(abi::add_registers(s[8], s[8], s[2]));
    builder.emit(abi::load_u32(s[5], s[8], 0));
    builder.emit(abi::load_u32(s[7], s[8], 4));
    builder.emit(abi::subtract_registers(s[5], s[5], s[7]));
    builder.emit(abi::store_u32(s[5], s[8], 0));
    builder.emit(abi::add_immediate(s[4], s[4], 1));
    builder.emit(abi::branch(&rewind_head));

    // --- no bands: the shader loops over every edge -------------------------------
    builder.emit(abi::label(&none));
    builder.emit(abi::move_immediate(s[4], "Integer", "0"));
    for field in [
        ITEM_BAND_TOP,
        ITEM_BAND_HEIGHT,
        ITEM_BAND_COUNT,
        ITEM_BAND_START,
    ] {
        builder.emit(abi::store_u32(s[4], sp(), block(field)));
    }
    builder.emit(abi::label(&done));
}

/// The bands the 16.16 edge at `SCRATCH[0]` goes in: first in `SCRATCH[4]`, last in
/// `SCRATCH[5]` (inclusive), from the working state `emit_band_index` parked. Uses
/// `SCRATCH[6]` as a temporary.
///
/// With `top = ymin − r`: first = ⌊(ylo − ymin) / h⌋ and last = ⌊(yhi − ymin + 2r) / h⌋.
/// Both numerators are non-negative (every y is at least ymin), so the unsigned divide
/// is exact, and the last is at most ⌊span / h⌋ = bands − 1.
fn emit_band_range(builder: &mut CodeBuilder, at: &BandSlots) {
    let s = abi::SCRATCH;
    let ordered = builder.label("vk_band_ordered");
    builder.emit(abi::load_u32(s[4], s[0], 4));
    builder.emit(abi::sign_extend_word(s[4], s[4]));
    builder.emit(abi::load_u32(s[5], s[0], 12));
    builder.emit(abi::sign_extend_word(s[5], s[5]));
    builder.emit(abi::compare_registers(s[4], s[5]));
    builder.emit(abi::branch_le(&ordered));
    builder.emit(abi::move_register(s[6], s[4]));
    builder.emit(abi::move_register(s[4], s[5]));
    builder.emit(abi::move_register(s[5], s[6]));
    builder.emit(abi::label(&ordered));
    builder.emit(abi::load_u64(s[6], abi::stack_pointer(), at.ymin));
    builder.emit(abi::subtract_registers(s[4], s[4], s[6]));
    builder.emit(abi::subtract_registers(s[5], s[5], s[6]));
    builder.emit(abi::load_u64(s[6], abi::stack_pointer(), at.reach));
    builder.emit(abi::add_registers(s[5], s[5], s[6]));
    builder.emit(abi::load_u64(s[6], abi::stack_pointer(), at.height));
    builder.emit(abi::unsigned_divide_registers(s[4], s[4], s[6]));
    builder.emit(abi::unsigned_divide_registers(s[5], s[5], s[6]));
}

/// Copy a `canvas::Picture`'s texels into the frame buffer's glyph region, and name
/// the slice in the item block (bug-484) — once per distinct image per frame (bug-688).
///
/// A picture is a rectangle whose fill colour is sampled from an image, so its block is
/// the rectangle's in every field but three, which it names exactly as a glyph does:
/// width in `misc.w`, height in `arc.x` (`ITEM_ARC_GLYPH_HEIGHT`), and the slice's base
/// in `arc.z` (`ITEM_ARC_EDGE_BASE`). The region, the cursor and the frame cap are the
/// glyphs' — the predicates count a picture's texels against that same cap.
///
/// **Once per distinct pixel block** (bug-688, Metal's bug-686 design): the block is
/// looked up in this frame's picture table first (`emit_picture_lookup`), and a hit
/// points the item at the texels already uploaded and copies nothing. A tilemap of 2,000
/// tiles drawn from two images copied 2,000 images and counted 2M texels against the
/// frame cap; it now copies two. The pass walk (`__canvas_passNext`) counts each
/// distinct block once a pass to match, and the table is a pass's: each pass is one
/// call of this renderer, which starts it empty.
///
/// **Must run after `emit_edge_upload`**, which zeroes `misc.w` and `arc.z` for every
/// kind that is not a polygon, and **before `emit_split_or_publish`**, so both records
/// of a split item carry the slice. Unlike a glyph run, a picture is one quad and does
/// not end the instanced run.
///
/// One 32-bit word per texel, copied verbatim: the pixel block is a `List OF Byte` of
/// RGBA8, so each little-endian word already reads `r | g << 8 | b << 16 | a << 24`,
/// the packing the shader unpacks. The block's address arrives split across two header
/// doubles (`HEADER_PICTURE_SHADOW_HI`/`_LO`) because a whole address overflows the
/// header hash; the texels start `COLLECTION_HEADER_SIZE` bytes in. The block is never
/// freed, so reading it here, during the frame, is safe. A zero address or a block
/// shorter than the width and height promise draws nothing — the bound
/// `canvas::shadowTexel` applies per read, applied once, as Metal's emitter does.
///
/// A picture whose texels would not fit the frame's remaining glyph room draws nothing
/// (`misc.w = 0`), rather than writing past the region. Unreachable: the predicate
/// declined such a frame to software.
///
/// Registers: `SCRATCH[0..=8]` only, and no calls. No `compare_immediate` or
/// `add_immediate` against a constant wider than 12 bits while `SCRATCH[7]` or
/// `SCRATCH[8]` is live: on AArch64 their encoding borrows x16/x17, which ARE those two.
fn emit_picture_upload(builder: &mut CodeBuilder, at: PictureSlots) {
    let done = builder.label("vk_pic_done");
    let empty = builder.label("vk_pic_empty");
    let copy_head = builder.label("vk_pic_copy_head");

    let header = abi::SCRATCH[6];
    let width = abi::SCRATCH[2];
    let height = abi::SCRATCH[3];
    let samples = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let block = abi::SCRATCH[7];

    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_PICTURE));
    let image = builder.label("vk_pic_image");
    builder.emit(abi::branch_eq(&image));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_TILE));
    builder.emit(abi::branch_ne(&done));
    builder.emit(abi::label(&image));

    builder.emit(abi::load_u64(header, abi::stack_pointer(), at.header));
    // A Tile reuses the gradient words for its source rectangle. Picture never reads
    // them, and Tile has no gradient, so the per-kind alias does not widen ItemBlock.
    let source_done = builder.label("vk_tile_source_done");
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_TILE));
    builder.emit(abi::branch_ne(&source_done));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        FIXED_POINT_SCALE,
    ));
    builder.emit(abi::signed_convert_to_float_d(
        abi::FP_SCRATCH[0],
        abi::SCRATCH[1],
    ));
    for (index, slot) in [TILE_SOURCE_X, TILE_SOURCE_Y, TILE_SOURCE_W, TILE_SOURCE_H]
        .into_iter()
        .enumerate()
    {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_multiply_d(
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[1],
            abi::FP_SCRATCH[0],
        ));
        builder.emit(abi::float_round_to_signed_x(
            abi::SCRATCH[1],
            abi::FP_SCRATCH[1],
        ));
        builder.emit(abi::store_u32(
            abi::SCRATCH[1],
            abi::stack_pointer(),
            at.item + ITEM_OFFSET_GRADIENT + index * 4,
        ));
    }
    builder.emit(abi::label(&source_done));
    for (slot, register) in [(HEADER_AUX0, width), (HEADER_AUX1, height)] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        builder.emit(abi::float_convert_to_signed_x(register, abi::FP_SCRATCH[1]));
    }
    builder.emit(abi::compare_immediate(width, "0"));
    builder.emit(abi::branch_le(&empty));
    builder.emit(abi::compare_immediate(height, "0"));
    builder.emit(abi::branch_le(&empty));
    builder.emit(abi::multiply_registers(samples, width, height));

    // block = (hi << PICTURE_SHADOW_SPLIT_BITS) | lo
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_SHADOW_HI * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(block, abi::FP_SCRATCH[1]));
    builder.emit(abi::shift_left_immediate(
        block,
        block,
        PICTURE_SHADOW_SPLIT_BITS as u8,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_PICTURE_SHADOW_LO * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::or_registers(block, block, abi::SCRATCH[1]));
    builder.emit(abi::compare_immediate(block, "0"));
    builder.emit(abi::branch_eq(&empty));
    // The block must hold every texel the width and height promise.
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        block,
        COLLECTION_OFFSET_COUNT,
    ));
    builder.emit(abi::shift_left_immediate(abi::SCRATCH[0], samples, 2));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_gt(&empty));

    // The height is the image's whatever the lookup finds, so it is stored now and its
    // register is free for the lookup.
    builder.emit(abi::store_u32(
        height,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    // Has this frame already uploaded this block? Then name those texels and stop.
    emit_picture_lookup(builder, &at, &done);

    // Room check against the glyph region's frame cap — the same bound the glyph copy
    // applies, through a register because the cap does not fit an immediate compare.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        samples,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[8],
        "Integer",
        &VULKAN_MAX_FRAME_GLYPH_SAMPLES.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[1], abi::SCRATCH[8]));
    builder.emit(abi::branch_gt(&empty));

    // The slice: width and the pre-advance cursor (the height is already stored).
    builder.emit(abi::store_u32(
        width,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    // Record (block, base) so later pictures of this block reuse the texels. The base is
    // still in `SCRATCH[0]`.
    emit_picture_record(builder, &at);

    // target = mapped + (GLYPH_BASE + base) * 4, the base being the advanced cursor less
    // `samples` (the record used the registers that held it).
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::subtract_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        samples,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &VULKAN_GLYPH_BASE_WORDS.to_string(),
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        2,
    ));
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, abi::SCRATCH[0]));
    builder.emit(abi::add_immediate(block, block, COLLECTION_HEADER_SIZE));

    // `samples` counts down; it is at least 1 here.
    builder.emit(abi::label(&copy_head));
    builder.emit(abi::load_u32(abi::SCRATCH[0], block, 0));
    builder.emit(abi::store_u32(abi::SCRATCH[0], target, 0));
    builder.emit(abi::add_immediate(block, block, 4));
    builder.emit(abi::add_immediate(target, target, 4));
    builder.emit(abi::subtract_immediate(samples, samples, 1));
    builder.emit(abi::compare_immediate(samples, "0"));
    builder.emit(abi::branch_gt(&copy_head));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&empty));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for offset in [
        ITEM_OFFSET_MISC + 12,
        ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
        ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ] {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.item + offset,
        ));
    }

    builder.emit(abi::label(&done));
}

/// The stack slots `emit_picture_upload` and its table helpers read.
struct PictureSlots {
    state: usize,
    item: usize,
    header: usize,
    glyph_cursor: usize,
    /// This frame's picture-table record count, zeroed with the other cursors.
    pic_count: usize,
    /// The empty index slot a missed lookup ended at, for `emit_picture_record`.
    pic_slot: usize,
}

/// The picture table's multiplicative hash constant (xorshift*'s) — the one Metal's
/// table uses — odd so the multiply is a bijection on the address bits it keeps.
const PICTURE_HASH_MULTIPLIER: &str = "2685821657736338717";

/// Look the picture's pixel block up in this frame's picture table (bug-688; the layout
/// is at `VULKAN_PICTURE_RECORDS_OFFSET`, Metal's design). On a hit, name the uploaded
/// texels in the item block (base in `arc.z`, width in `misc.w`) and branch to `done`;
/// on a miss, park the empty index slot the probe ended at in `pic_slot` and fall
/// through to the upload.
///
/// Live on entry: `block` (`SCRATCH[7]`), `width` (`SCRATCH[2]`), `samples`
/// (`SCRATCH[4]`). Uses `SCRATCH[0,1,3,5,6,8]`.
///
/// A slot is live only when its record number is below this frame's count and that
/// record names the slot back — the sparse-set test — so the index never needs clearing,
/// and whatever the mapped memory held before the first frame reads as empty.
fn emit_picture_lookup(builder: &mut CodeBuilder, at: &PictureSlots, done: &str) {
    let probe = builder.label("vk_pic_probe");
    let live = builder.label("vk_pic_live");
    let miss = builder.label("vk_pic_miss");
    let hit = builder.label("vk_pic_hit");
    let (index, record_no, record, records, slot, count) = (
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[8],
    );
    let (width, block) = (abi::SCRATCH[2], abi::SCRATCH[7]);
    const INDEX_BITS: u32 = VULKAN_PICTURE_INDEX_SLOTS.trailing_zeros();

    // records = mapped + RECORDS_OFFSET; index = mapped + INDEX_OFFSET.
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        records,
    );
    builder.emit(abi::move_immediate(
        index,
        "Integer",
        &VULKAN_PICTURE_INDEX_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(index, records, index));
    builder.emit(abi::move_immediate(
        record_no,
        "Integer",
        &VULKAN_PICTURE_RECORDS_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(records, records, record_no));
    // slot = ((block >> 4) * K) >> (64 - INDEX_BITS). Blocks are 16-byte aligned arena
    // allocations, so the low four bits carry nothing.
    builder.emit(abi::shift_right_immediate(slot, block, 4));
    builder.emit(abi::move_immediate(
        record_no,
        "Integer",
        PICTURE_HASH_MULTIPLIER,
    ));
    builder.emit(abi::multiply_registers(slot, slot, record_no));
    builder.emit(abi::shift_right_immediate(
        slot,
        slot,
        (64 - INDEX_BITS) as u8,
    ));
    builder.emit(abi::load_u64(count, abi::stack_pointer(), at.pic_count));

    builder.emit(abi::label(&probe));
    builder.emit(abi::shift_left_immediate(record_no, slot, 2));
    builder.emit(abi::add_registers(record_no, index, record_no));
    builder.emit(abi::load_u32(record_no, record_no, 0));
    // Unsigned: a slot holding garbage from before this buffer's first frame is a large
    // number, and it must read as "not below the count" rather than as negative.
    builder.emit(abi::compare_registers(record_no, count));
    builder.emit(abi::branch_lo(&live));
    builder.emit(abi::branch(&miss));
    builder.emit(abi::label(&live));
    const _: () = assert!(
        PICTURE_RECORD_BYTES == 16,
        "records are addressed by a shift of 4"
    );
    builder.emit(abi::shift_left_immediate(record, record_no, 4));
    builder.emit(abi::add_registers(record, records, record));
    builder.emit(abi::load_u32(record_no, record, 12));
    builder.emit(abi::compare_registers(record_no, slot));
    builder.emit(abi::branch_ne(&miss));
    builder.emit(abi::load_u64(record_no, record, 0));
    builder.emit(abi::compare_registers(record_no, block));
    builder.emit(abi::branch_eq(&hit));
    // Another block's live slot: the next one, wrapping.
    builder.emit(abi::add_immediate(slot, slot, 1));
    builder.emit(abi::move_immediate(
        record_no,
        "Integer",
        &(VULKAN_PICTURE_INDEX_SLOTS - 1).to_string(),
    ));
    builder.emit(abi::and_registers(slot, slot, record_no));
    builder.emit(abi::branch(&probe));

    builder.emit(abi::label(&hit));
    builder.emit(abi::load_u32(record_no, record, 8));
    builder.emit(abi::store_u32(
        record_no,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));
    builder.emit(abi::store_u32(
        width,
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::branch(done));

    builder.emit(abi::label(&miss));
    builder.emit(abi::store_u64(slot, abi::stack_pointer(), at.pic_slot));
}

/// Record the picture just given its slice of the glyph region — `block` in
/// `SCRATCH[7]`, its base (the pre-advance glyph cursor) in `SCRATCH[0]` — at the index
/// slot `emit_picture_lookup` parked (bug-688).
///
/// Uses `SCRATCH[0,1,3,5,6]`; `block`, `width` and `samples` survive for the copy.
/// The count check is unreachable — one record per picture item, and the predicate caps
/// items at `VULKAN_PICTURE_RECORDS` — and is kept because the alternative is a write
/// past the table; an unrecorded picture is still uploaded, only not shared.
fn emit_picture_record(builder: &mut CodeBuilder, at: &PictureSlots) {
    let full = builder.label("vk_pic_table_full");
    let (base, count, entry, temp, mapped) = (
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    );
    let block = abi::SCRATCH[7];

    builder.emit(abi::load_u64(count, abi::stack_pointer(), at.pic_count));
    builder.emit(abi::move_immediate(
        temp,
        "Integer",
        &VULKAN_PICTURE_RECORDS.to_string(),
    ));
    builder.emit(abi::compare_registers(count, temp));
    builder.emit(abi::branch_ge(&full));

    // records[count] = (block, base, slot)
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        mapped,
    );
    builder.emit(abi::shift_left_immediate(entry, count, 4));
    builder.emit(abi::add_registers(entry, mapped, entry));
    builder.emit(abi::move_immediate(
        temp,
        "Integer",
        &VULKAN_PICTURE_RECORDS_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(entry, entry, temp));
    builder.emit(abi::store_u64(block, entry, 0));
    builder.emit(abi::store_u32(base, entry, 8));
    builder.emit(abi::load_u64(temp, abi::stack_pointer(), at.pic_slot));
    builder.emit(abi::store_u32(temp, entry, 12));
    // index[slot] = count -- written AFTER the record, so the slot is never live while
    // the record it names is stale.
    builder.emit(abi::shift_left_immediate(temp, temp, 2));
    builder.emit(abi::add_registers(entry, mapped, temp));
    builder.emit(abi::move_immediate(
        temp,
        "Integer",
        &VULKAN_PICTURE_INDEX_OFFSET.to_string(),
    ));
    builder.emit(abi::add_registers(entry, entry, temp));
    builder.emit(abi::store_u32(count, entry, 0));
    builder.emit(abi::add_immediate(count, count, 1));
    builder.emit(abi::store_u64(count, abi::stack_pointer(), at.pic_count));
    builder.emit(abi::label(&full));
}

/// Copy the item block just built on the stack into the frame's item buffer at the
/// cursor, and advance the cursor by one quad.
///
/// This is what replaced `vkCmdPushConstants` (plan-116-A). The block is built on the
/// stack exactly as before — `emit_item_block` and `emit_edge_upload` are untouched —
/// and then lands in the buffer instead of in the command stream, at the index the
/// shaders will read it back from through `gl_InstanceIndex`.
///
/// **The cursor counts quads, not scene items.** A shape is one, and a glyph run is
/// one per glyph, because each glyph is its own quad with its own block. That is the
/// same number `__canvas_blockCost` sums against `VULKAN_MAX_FRAME_ITEMS`, so
/// the two cannot disagree about what "full" means.
///
/// Branches to `full` without writing or advancing if the buffer is already at
/// capacity. Unreachable in the same way `emit_edge_upload`'s bound is: the pass walk
/// ends a pass before its quads pass the region (bug-765). Kept because the
/// alternative is a write past the mapping.
fn emit_item_publish(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_item_cursor: usize,
    full: &str,
) {
    let cursor = abi::SCRATCH[0];
    let target = abi::SCRATCH[1];
    let value = abi::SCRATCH[2];
    let stride = abi::SCRATCH[3];

    builder.emit(abi::load_u64(cursor, abi::stack_pointer(), off_item_cursor));
    builder.emit(abi::compare_immediate(
        cursor,
        &VULKAN_MAX_FRAME_ITEMS.to_string(),
    ));
    builder.emit(abi::branch_ge(full));

    // target = mapped + cursor * ITEM_BLOCK_SIZE. A multiply rather than a second
    // byte-cursor kept in step with this one: two cursors that must never diverge is
    // exactly the invariant that breaks silently, and the product is one instruction.
    builder.emit(abi::move_immediate(
        stride,
        "Integer",
        &ITEM_BLOCK_SIZE.to_string(),
    ));
    builder.emit(abi::multiply_registers(stride, cursor, stride));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_ITEM_MAPPED,
        target,
    );
    builder.emit(abi::add_registers(target, target, stride));

    // 112 bytes is 14 words. Unrolled: a loop would need a counter register that no
    // call clobbers, for fourteen iterations.
    // bug-550: a `const` assertion, not a `debug_assert_eq!`. Both operands are
    // compile-time constants, so this is decidable when the compiler ITSELF is
    // built and costs nothing at run time — strictly better than an assertion that
    // ran nowhere because CI builds `--release`.
    const _: () = assert!(
        ITEM_BLOCK_SIZE % 8 == 0,
        "the item block is copied to the buffer eight bytes at a time"
    );
    for word in 0..ITEM_BLOCK_SIZE / 8 {
        builder.emit(abi::load_u64(
            value,
            abi::stack_pointer(),
            off_item + word * 8,
        ));
        builder.emit(abi::store_u64(value, target, word * 8));
    }

    builder.emit(abi::add_immediate(cursor, cursor, 1));
    builder.emit(abi::store_u64(
        cursor,
        abi::stack_pointer(),
        off_item_cursor,
    ));
}

/// Publish this item's block — as **two** records when the fragment shader's
/// stroke-over-fill composition would not equal the oracle's two sequential blends.
///
/// The shaders compose stroke over fill *in the shader* and hand the hardware one
/// source, and `mfb_canvas.frag` states the identity that rests on: it equals the
/// oracle's two writes because `over` is associative. **That is `Normal`-only.** The
/// oracle applies the mode twice per pixel — fill into the surface, then stroke into
/// the result — and `M(M(D, fill), stroke) = M(D, over(stroke, fill))` holds for
/// `over` and for none of `Multiply`, `Screen` or `Add` wherever the stroke band
/// covers filled pixels.
///
/// So an item that is non-`Normal` **and** both fills and strokes becomes two adjacent
/// records: the first with `strokeHalf` zeroed (fill only), the second with the fill
/// alpha zeroed (stroke only), in that order. Each then reaches the fixed-function
/// unit as a single source, and paint order is exactly the oracle's. The fragment
/// shader needs no change for it — a zero `strokeHalf` skips the stroke arm and a zero
/// fill alpha premultiplies to nothing.
///
/// Everything else stays one record: `Normal` items, fill-only items and stroke-only
/// items all take the single-publish path they had.
fn emit_split_or_publish(
    builder: &mut CodeBuilder,
    off_state: usize,
    off_item: usize,
    off_item_cursor: usize,
    off_item_mode: usize,
    off_saved_stroke: usize,
    full: &str,
) {
    let single = builder.label("vk_publish_single");
    let done = builder.label("vk_publish_done");
    let strokes = builder.label("vk_publish_strokes");

    // Split only when the mode is not Normal...
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item_mode,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&single));
    // ...and the item actually strokes: `strokeHalf` > 0, which since bug-735 is its
    // 16.16 word > 0 or, for a word of 0, its residual > 0 -- `emit_item_stroke_sign`
    // makes that exactly the header's f64 `half > 0.0`, the test
    // `__canvas_blockInstances` counts two records on. Before it this read the word
    // alone, so a blended filled item stroked thinner than 2^-16 px was counted as two
    // records and published as one, and every later draw named the wrong blocks.
    //
    // SIGNED, and that is the whole point. `__canvas_strokeHalf` returns **-1.0** for a
    // paint that does not stroke, which is `0xFFFF0000` in 16.16 -- and `load_u32`
    // zero-extends, so the 64-bit compare below saw 4294901760 and every blended
    // fill-only item took the SPLIT path. The comment under `single` has always claimed
    // such an item is fill-only and takes the single path; this is the load that makes
    // it true.
    //
    // It stayed invisible until plan-116-H. While the draws came from `emit_run_flush`
    // the instance count was `cursor - run_start` -- whatever had actually been
    // published -- so the spurious second record was drawn and painted nothing. Once the
    // draw list predicts instance counts independently (`__canvas_blockInstances`), one
    // extra record shifts every later base by one and the scene silently loses its tail.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    builder.emit(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_gt(&strokes));
    builder.emit(abi::branch_lt(&single));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC_LO + 4,
    ));
    builder.emit(abi::sign_extend_word(abi::SCRATCH[0], abi::SCRATCH[0]));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_le(&single));
    builder.emit(abi::label(&strokes));
    // ...and actually fills (fill alpha > 0). A `Line` or an `Arc` reaches here with
    // its stroke colour already moved into the fill slots and `strokeHalf` negative
    // (`__canvas_strokeAsFill`), so it is fill-only and takes the single path.
    //
    // plan-163-D: "fills" is a visible fill alpha, OR a gradient (its ramp is the fill
    // colour whatever the fill alpha), OR a picture (its image is, and since plan-163-C
    // its fill alpha is only the tint's strength). Testing the fill alpha alone sent a
    // blended stroked gradient or picture with a transparent fill down the single path,
    // composing stroke over fill in the shader under a mode where that is wrong.
    let fills = builder.label("vk_publish_fills");
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_FILL + 12,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_gt(&fills));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_ELLIPSE + ITEM_ELLIPSE_GRADIENT_COUNT,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "2"));
    builder.emit(abi::branch_ge(&fills));
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(
        abi::SCRATCH[0],
        &GEO_KIND_PICTURE.to_string(),
    ));
    builder.emit(abi::branch_ne(&single));
    builder.emit(abi::label(&fills));

    // Record one: the fill, with the stroke switched off.
    //
    // `strokeHalf` is parked on the STACK, not in a scratch register.
    // `emit_item_publish` uses `SCRATCH[0..3]` — `SCRATCH[1]` is its target pointer —
    // so a register saved across it comes back holding a mapped address. Measured
    // rather than reasoned about: doing exactly that gave record two a `strokeHalf` of
    // whatever the buffer pointer was, i.e. an enormous stroke band, and the Vulkan
    // harness reported `worst=215` with the GPU painting stroke pixels the oracle
    // never touched.
    //
    // Off is -1.0 (`0xFFFF0000`), the value `__canvas_strokeHalf` reports for a paint
    // that does not stroke: since bug-735 the shader strokes on the word PLUS its
    // residual, and a zero word under a positive residual would still stroke. Spelled
    // unsigned: the store keeps the low 32 bits.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_saved_stroke,
    ));
    // -1.0 in 16.16, spelled unsigned: the immediate is never negative, and the store
    // keeps the low 32 bits.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        "4294901760",
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    emit_item_publish(builder, off_state, off_item, off_item_cursor, full);
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_saved_stroke,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));

    // Record two: the stroke, with the fill made fully transparent.
    // plan-163-D: marked OFF -- alpha -1, which every shader reads as "no fill in this
    // record" -- rather than made transparent: a gradient's or a picture's colour does
    // not come from the fill alpha, so a zero there left record two drawing the ramp or
    // the image a second time under the blend. The stroke still reads the ramp
    // (`FillAndStroke`). Spelled unsigned; the store keeps the low 32 bits.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        "4294967295",
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_FILL + 12,
    ));
    emit_item_publish(builder, off_state, off_item, off_item_cursor, full);
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&single));
    emit_item_publish(builder, off_state, off_item, off_item_cursor, full);
    builder.emit(abi::label(&done));
}

/// Draw every quad published since the last flush as **one instanced draw**, and start
/// a new run.
///
/// Walk `__CANVAS_DRAWS` and issue one instanced `vkCmdDraw` per entry, pushing that
/// entry's group offset first (plan-116-H).
///
/// **This is the second of the emitter's two passes** (**H9**). The first fills the item
/// buffer, walking the block list once so a shared group's blocks are written once; this
/// one walks the draw list, which may visit the same block range several times — a
/// diamond is two entries with the same base. No single forward pass can do both, which
/// is what the Phase 1 sharing decision costs and buys.
///
/// Every entry is one instanced call, text included. A glyph run is N contiguous blocks
/// in the item buffer, one per quad, so `instanceCount = N` draws them all — the
/// "a glyph run is N draws by design" shape predates plan-116-A putting the block in a
/// buffer indexed by instance, and the draw list's counts are already in instances.
///
/// Each entry is four `i32`s: `base`, `count`, `dx`, `dy` — the offsets in 16.16, which
/// is the form the push constant wants, so they travel from MFBASIC to the shader
/// without a conversion on either side.
#[allow(clippy::too_many_arguments)]
fn emit_draw_list_pass(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_state: usize,
    off_handle: usize,
    off_cmd_handle: usize,
    off_draw_fn: usize,
    off_draws: usize,
    off_draw_entries: usize,
    unavailable: &str,
) -> Result<(), String> {
    let head = builder.label("vk_drawlist_head");
    let done = builder.label("vk_drawlist_done");

    let off_push_fn = builder.allocate_stack_object("vk_push_fn", 8);
    let off_index = builder.allocate_stack_object("vk_drawlist_index", 8);
    // The two 16.16 words, their residual words and (plan-172-H) the four words of the
    // inverse linear part, contiguous, so `pValues` can point straight at them.
    let off_offset_pair = builder.allocate_stack_object("vk_push_offset", PUSH_OFFSET_BYTES);

    let off_bind_fn = builder.allocate_stack_object("vk_drawlist_bind_fn", 8);
    for (name, slot) in [
        ("vkCmdPushConstants", off_push_fn),
        ("vkCmdBindPipeline", off_bind_fn),
    ] {
        emit_dlsym(
            builder,
            platform,
            platform_imports,
            name,
            off_handle,
            slot,
            unavailable,
        )?;
    }

    builder.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), off_index));
    builder.emit(abi::label(&head));
    let index = builder.temporary_vreg();
    let total = builder.temporary_vreg();
    builder.emit(abi::load_u64(&index, abi::stack_pointer(), off_index));
    builder.emit(abi::load_u64(
        &total,
        abi::stack_pointer(),
        off_draw_entries,
    ));
    builder.emit(abi::compare_registers(&index, &total));
    builder.emit(abi::branch_ge(&done));

    // entry = draws + index * (CANVAS_DRAW_ENTRY_WORDS * 8)
    let entry = builder.temporary_vreg();
    builder.emit(abi::load_u64(&entry, abi::stack_pointer(), off_draws));
    // Sixteen elements per entry, eight bytes each. An MFBASIC `Integer` is 64-bit,
    // which is the whole reason the fields below are at 0/8/16/24/32 and are loaded with
    // `load_u64`: reading them as 32-bit words takes the low half of `base` and the low
    // half of `count` as if they were `base` and `dx`, which draws real geometry at
    // nonsense instance indices -- 35% of the frame wrong, worst=255, with a draw list
    // that every assertion says is correct.
    let scaled = builder.temporary_vreg();
    builder.emit(abi::shift_left_immediate(
        &scaled,
        &index,
        CANVAS_DRAW_ENTRY_SHIFT as u8,
    ));
    builder.emit(abi::add_registers(&entry, &entry, &scaled));

    // The offset pair, copied into its own slot so `pValues` has a stable address that
    // is not part of a collection another thread may be writing.
    let word = builder.temporary_vreg();
    builder.emit(abi::load_u64(&word, &entry, 16));
    builder.emit(abi::store_u32(&word, abi::stack_pointer(), off_offset_pair));
    builder.emit(abi::load_u64(&word, &entry, 24));
    builder.emit(abi::store_u32(
        &word,
        abi::stack_pointer(),
        off_offset_pair + 4,
    ));

    // The base and count are staged through the stack for the reason `emit_run_flush`
    // records: on x86-64 the scratch pool aliases the C argument bank, so a value held
    // in a scratch register across the staging is one an earlier argument destroys.
    let off_base = builder.allocate_stack_object("vk_drawlist_base", 8);
    let off_count = builder.allocate_stack_object("vk_drawlist_count", 8);
    builder.emit(abi::load_u64(&word, &entry, 0));
    builder.emit(abi::store_u64(&word, abi::stack_pointer(), off_base));
    builder.emit(abi::load_u64(&word, &entry, 8));
    builder.emit(abi::store_u64(&word, abi::stack_pointer(), off_count));

    // Bind THIS entry's pipeline. The publish walk binds as it goes, but every draw now
    // happens after that walk has finished, so a binding left there would apply the last
    // item's mode to the whole frame — measured at 35.1% of pixels wrong before the mode
    // travelled with the entry.
    // handle = *(state + …_PIPELINE_MODES + mode * 8) — the same shift-and-add the
    // publish walk uses, so the two cannot pick different pipelines for one mode.
    //
    // In vregs, not `SCRATCH[k]`: `entry` is a vreg live across this stretch, and the
    // allocator does not see hand-assigned scratch, so on x86-64 it may put `entry` in
    // the very register a fixed `SCRATCH[k]` names (bug-688 measured that collision in
    // `emit_state_load`).
    let off_pipeline = builder.allocate_stack_object("vk_drawlist_pipeline", 8);
    let state = builder.temporary_vreg();
    builder.emit(abi::load_u64(&word, &entry, CANVAS_DRAW_ENTRY_MODE));
    builder.emit(abi::shift_left_immediate(&word, &word, 3));
    builder.emit(abi::load_u64(&state, abi::stack_pointer(), off_state));
    builder.emit(abi::add_registers(&word, &state, &word));
    builder.emit(abi::load_u64(
        &word,
        &word,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES,
    ));
    builder.emit(abi::store_u64(&word, abi::stack_pointer(), off_pipeline));
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    builder.emit(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_pipeline,
    ));
    emit_call_fn(builder, off_bind_fn);

    // Now the offset pair, (bug-735) its residual words, entry words 5 and 6, and
    // (plan-172-H) the inverse linear part, entry words 8-11, which are zero in an entry
    // with no matrix -- into the slot the bind above borrowed.
    for (word_index, at) in [
        (2usize, 0usize),
        (3, 4),
        (5, 8),
        (6, 12),
        (8, 16),
        (9, 20),
        (10, 24),
        (11, 28),
    ] {
        builder.emit(abi::load_u64(word, entry, word_index * 8));
        builder.emit(abi::store_u32(
            word,
            abi::stack_pointer(),
            off_offset_pair + at,
        ));
    }

    // vkCmdPushConstants(cmd, layout, stageFlags, offset, size, pValues)
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        SHADER_STAGE_VERTEX_AND_FRAGMENT,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    // Arguments five and six go through the register-model helpers, not through
    // `c_arg(4)` and `c_arg(5)` directly. `has_vulkan_backend` is true for **Windows**
    // as well as Linux, and Win64 passes only four integer arguments in registers — so
    // a raw `c_arg(4)` there writes a register the callee never reads and leaves the
    // real stack slot undefined. This is the only six-argument call this pass makes,
    // and it is the reason `emit_run_flush` reached for `emit_int_arg_slot` on its
    // fifth argument rather than writing the register.
    emit_int_arg(builder, platform, 4, &PUSH_OFFSET_BYTES.to_string());
    emit_addr_arg(builder, platform, 5, off_offset_pair);
    emit_call_fn(builder, off_push_fn);

    // vkCmdDraw(cmd, 4, count, 0, base)
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "4"));
    builder.emit(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_int_arg_slot(builder, platform, 4, off_base);
    emit_call_fn(builder, off_draw_fn);

    let bumped = builder.temporary_vreg();
    builder.emit(abi::load_u64(&bumped, abi::stack_pointer(), off_index));
    builder.emit(abi::add_immediate(&bumped, &bumped, 1));
    builder.emit(abi::store_u64(&bumped, abi::stack_pointer(), off_index));
    builder.emit(abi::branch(&head));

    builder.emit(abi::label(&done));
    Ok(())
}

/// `canvas::vulkanDrawScene(surface, width, height, geometry, offsets)` — render one
/// frame on the GPU and read it back into `surface`.
///
/// The Vulkan counterpart of `canvas::metalDrawScene`, and deliberately the same
/// shape: draw offscreen, read back, and let the frame leave through the same
/// `canvas::blitSurface` every other path uses. That is what makes the two backends
/// and the software oracle comparable at all — the tolerance comparator diffs an
/// RGBA8 buffer — and here it is also what makes the renderer testable headless.
///
/// This member always reads back. The direct present is `canvas::vulkanPresentScene`
/// (`vulkan_present.rs`), which is Windows only: on Linux a frame is read back even in a
/// window.
///
/// The submit is followed by `vkQueueWaitIdle`, so the frame is complete before this
/// returns and `canvas::frameDone` advances D's counter after real GPU completion —
/// the same ordering plan-98-E Correction 12 records for Metal, and for the same
/// reason: the readback is on the critical path, so a fence-and-continue would be
/// weaker, not stronger.
/// The stack slots `emit_glyph_draws` reads and writes.
///
/// A struct rather than seventeen parameters, because every one of them is a `usize`
/// stack offset and a transposed pair would compile, run, and draw the wrong thing.
struct GlyphPublishSlots {
    state: usize,
    item: usize,
    header: usize,
    width: usize,
    height: usize,
    /// The frame's item-buffer cursor — shared with the shape loop, because a glyph is
    /// a quad and takes a block exactly as a shape does.
    item_cursor: usize,
    /// Where this glyph's block landed, parked so the draw's `firstInstance` can be
    /// staged from memory rather than held in a scratch register across the staging.
    instance: usize,
    glyph_meta: usize,
    glyph_cov: usize,
    glyph_cursor: usize,
    glyph_index: usize,
    glyph_count: usize,
    glyph_w: usize,
    glyph_h: usize,
    glyph_x: usize,
    glyph_y: usize,
}

/// One quad per glyph, for a `__CANVAS_GEO_TEXT` item.
///
/// **Why a glyph is not a shape.** Every other kind is one draw whose fragment shader
/// evaluates a signed distance. A glyph has no distance to evaluate: the CPU already
/// rasterised its outline into a coverage bitmap and cached it, which is the whole point
/// of plan-98-G Phase 2 — the alternative costs `O(edges)` per pixel and measured 8.1
/// seconds for twelve characters. So the GPU's job is a *lookup*, and the unit of work
/// is the glyph rather than the run.
///
/// The bitmap travels the same way a polygon's edges do: copied into the frame's shared
/// buffer at a running cursor, with the offset in the item block. One sample per 32-bit
/// word — the region is sized for it, and the arm that reads it has no shifting to get
/// wrong (`VULKAN_MAX_FRAME_GLYPH_SAMPLES`).
///
/// Everything the loop carries lives on the stack, not in registers. Two calls happen
/// per glyph, and on x86-64 the scratch pool aliases the C argument bank — so a loop
/// counter in a register is a loop counter the next call overwrites (`.ai/arch-abi.md`).
fn emit_glyph_publish(builder: &mut CodeBuilder, at: GlyphPublishSlots) {
    let head = builder.label("vk_glyph_head");
    let done = builder.label("vk_glyph_done");
    let next = builder.label("vk_glyph_next");
    let blank = builder.label("vk_glyph_blank");
    let publish = builder.label("vk_glyph_publish");
    let copy_head = builder.label("vk_glyph_copy_head");
    let copy_done = builder.label("vk_glyph_copy_done");

    // The run's glyph count, and the colour/surface fields, come from the item block the
    // header describes — built once for the run and then edited per glyph, because fill,
    // stroke and surface are the same for every glyph in it.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.header,
    ));
    emit_item_block(builder, at.item, at.width, at.height);

    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.header,
    ));
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        abi::SCRATCH[0],
        HEADER_AUX0 * 8,
    ));
    builder.emit(abi::float_convert_to_signed_x(
        abi::SCRATCH[1],
        abi::FP_SCRATCH[1],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_count,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[1], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_index,
    ));

    // kind = 6, radius = 0, strokeHalf = 0. A glyph is fill-only: a stroked text item
    // became an outline polygon in the geometry builder, so it never reaches here.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        GEO_KIND_TEXT,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for word in 1..3 {
        builder.emit(abi::store_u32(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            at.item + ITEM_OFFSET_MISC + word * 4,
        ));
    }

    builder.emit(abi::label(&head));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_index,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_count,
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[0], abi::SCRATCH[1]));
    builder.emit(abi::branch_ge(&done));

    // run = header + HEADER_SLOTS + index * GLYPH_RUN_SLOTS, in doubles.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &GLYPH_RUN_SLOTS.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.header,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[1],
        abi::SCRATCH[0],
    ));
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        HEADER_SLOTS * 8,
    ));

    // entry, penX, penY.
    for (slot, register) in [
        (0usize, abi::SCRATCH[2]),
        (1, abi::SCRATCH[3]),
        (2, abi::SCRATCH[4]),
    ] {
        builder.emit(abi::load_double(
            abi::FP_SCRATCH[1],
            abi::SCRATCH[0],
            slot * 8,
        ));
        builder.emit(abi::float_convert_to_signed_x(register, abi::FP_SCRATCH[1]));
    }
    // A cache entry of -1 is a glyph the eviction pass dropped after this run was
    // built. It draws nothing rather than reading the metadata list out of range, and
    // still publishes its (blank) block — see `blank` below.
    builder.emit(abi::compare_immediate(abi::SCRATCH[2], "0"));
    builder.emit(abi::branch_lt(&blank));

    // meta = glyphMeta + entry * GLYPH_META_SLOTS, in 8-byte Integers.
    builder.emit(abi::move_immediate(
        abi::SCRATCH[5],
        "Integer",
        &GLYPH_META_SLOTS.to_string(),
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        abi::SCRATCH[5],
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[2],
        abi::SCRATCH[2],
        3,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_meta,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[2],
        abi::SCRATCH[5],
        abi::SCRATCH[2],
    ));

    // x = x0 + penX, y = y0 + penY; w, h; and the coverage start, parked for the copy.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_X0 * 8,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[3],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_x,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::SCRATCH[2],
        GLYPH_META_Y0 * 8,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[5],
        abi::SCRATCH[4],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_y,
    ));
    for (slot, parked) in [(GLYPH_META_W, at.glyph_w), (GLYPH_META_H, at.glyph_h)] {
        builder.emit(abi::load_u64(abi::SCRATCH[5], abi::SCRATCH[2], slot * 8));
        builder.emit(abi::store_u64(
            abi::SCRATCH[5],
            abi::stack_pointer(),
            parked,
        ));
    }
    // An empty bitmap — a space, or a glyph with no contours — has nothing to copy and
    // nothing to draw, but still publishes its (blank) block — see `blank` below.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[5], "0"));
    builder.emit(abi::branch_le(&blank));
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[6], "0"));
    builder.emit(abi::branch_le(&blank));

    // samples = w * h, and the frame's remaining room for them. The predicate has
    // already declined a frame that does not fit, so this bound is the emitter refusing
    // to write past its buffer if the two ever disagree — not a policy of its own.
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[8],
        abi::SCRATCH[7],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[9],
        "Integer",
        &VULKAN_MAX_FRAME_GLYPH_SAMPLES.to_string(),
    ));
    builder.emit(abi::compare_registers(abi::SCRATCH[8], abi::SCRATCH[9]));
    builder.emit(abi::branch_gt(&blank));

    // --- copy the bitmap into the buffer's glyph region --------------------------
    // dst = mapped + (GLYPH_BASE + cursor) * 4, src = coverage + covStart.
    builder.emit(abi::load_u64(
        abi::SCRATCH[8],
        abi::SCRATCH[2],
        GLYPH_META_START * 8,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[9],
        abi::stack_pointer(),
        at.glyph_cov,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[8],
        abi::SCRATCH[9],
        abi::SCRATCH[8],
    ));
    emit_state_load(
        builder,
        at.state,
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        abi::SCRATCH[9],
    );
    builder.emit(abi::add_immediate(
        abi::SCRATCH[10],
        abi::SCRATCH[7],
        VULKAN_GLYPH_BASE_WORDS,
    ));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[10],
        abi::SCRATCH[10],
        2,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[9],
        abi::SCRATCH[9],
        abi::SCRATCH[10],
    ));
    builder.emit(abi::move_immediate(abi::SCRATCH[10], "Integer", "0"));
    builder.emit(abi::label(&copy_head));
    builder.emit(abi::compare_registers(abi::SCRATCH[10], abi::SCRATCH[6]));
    builder.emit(abi::branch_ge(&copy_done));
    builder.emit(abi::add_registers(
        abi::SCRATCH[11],
        abi::SCRATCH[8],
        abi::SCRATCH[10],
    ));
    builder.emit(abi::load_u8(abi::SCRATCH[11], abi::SCRATCH[11], 0));
    builder.emit(abi::shift_left_immediate(
        abi::SCRATCH[12],
        abi::SCRATCH[10],
        2,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[12],
        abi::SCRATCH[9],
        abi::SCRATCH[12],
    ));
    builder.emit(abi::store_u32(abi::SCRATCH[11], abi::SCRATCH[12], 0));
    builder.emit(abi::add_immediate(abi::SCRATCH[10], abi::SCRATCH[10], 1));
    builder.emit(abi::branch(&copy_head));
    builder.emit(abi::label(&copy_done));

    // --- the glyph's own item block ----------------------------------------------
    // quad is 16.16 like every other kind's; shape.x/.y are WHOLE pixels, because the
    // shader indexes the bitmap with them and a fixed-point origin would have to be
    // converted back — losing the exactness that makes the lookup a lookup.
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_x,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.glyph_y,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_SHAPE,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_SHAPE + 4,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[4],
        abi::SCRATCH[0],
        abi::SCRATCH[2],
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[5],
        abi::SCRATCH[1],
        abi::SCRATCH[3],
    ));
    // The per-glyph quad narrows the item's quad to this one glyph's box, so a run of
    // twenty glyphs rasterises twenty small quads instead of twenty copies of the whole
    // run's box. That narrowing is only valid UNTRANSFORMED: the box is in shape space,
    // and under a transform the glyph's pixels are somewhere else entirely — the GPU
    // would rasterise a region the glyph no longer occupies and draw nothing.
    //
    // Transformed, the item's quad is left as `emit_item_block` wrote it, which is the
    // whole run's transformed hull (`__canvas_boundsHeader` maps the four corners
    // forward). Correct, and the cost is stated rather than hidden: every glyph in a
    // transformed run rasterises the run's hull, so a long transformed string is
    // O(glyphs × hull) fragments. Narrowing it would mean forward-mapping four corners
    // in emitted machine code, in both backends, to save fragments on a case that is
    // not the common one.
    {
        let keep_hull = builder.label("vk_glyph_hull");
        builder.emit(abi::load_u32(
            abi::SCRATCH[6],
            abi::stack_pointer(),
            at.item + ITEM_OFFSET_TRANSFORM + 24,
        ));
        builder.emit(abi::compare_immediate(abi::SCRATCH[6], "0"));
        builder.emit(abi::branch_ne(&keep_hull));
        for (register, word) in [
            (abi::SCRATCH[0], 0usize),
            (abi::SCRATCH[1], 1),
            (abi::SCRATCH[4], 2),
            (abi::SCRATCH[5], 3),
        ] {
            builder.emit(abi::shift_left_immediate(abi::SCRATCH[6], register, 16));
            builder.emit(abi::store_u32(
                abi::SCRATCH[6],
                abi::stack_pointer(),
                at.item + ITEM_OFFSET_QUAD + word * 4,
            ));
        }
        builder.emit(abi::label(&keep_hull));
    }
    // misc.w = width, arc.x = height, arc.z = the sample offset into the region.
    builder.emit(abi::store_u32(
        abi::SCRATCH[2],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_EDGE_BASE,
    ));

    // Advance the cursor before the calls: `w * h` is in a scratch register the calls
    // are free to clobber.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::multiply_registers(
        abi::SCRATCH[6],
        abi::SCRATCH[5],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::add_registers(
        abi::SCRATCH[7],
        abi::SCRATCH[7],
        abi::SCRATCH[6],
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[7],
        abi::stack_pointer(),
        at.glyph_cursor,
    ));

    // --- publish and draw ------------------------------------------------------------
    // This glyph's block goes into the frame's item buffer like any other quad's, and
    // the draw names it through `firstInstance`. The index is parked *before* the
    // publish, because publishing advances the cursor past it.
    builder.emit(abi::label(&publish));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item_cursor,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.instance,
    ));
    emit_item_publish(builder, at.state, at.item, at.item_cursor, &next);
    builder.emit(abi::branch(&next));

    // A glyph that draws nothing — a space, an evicted cache entry, a bitmap past the
    // frame's glyph region — still publishes ONE block, with width and height 0, so the
    // shader's bounds test in `glyphCoverage` answers zero coverage everywhere (bug-484).
    //
    // Skipping the publish desynchronised the item buffer from the draw list:
    // `__canvas_blockInstances` counts a text run as its full glyph count, so every draw
    // entry after a run containing a space named blocks one position on — the next items
    // drew under their neighbours' pipelines and group offsets, and the last read a block
    // never written. Found on Metal by bug-484's picture scene ("Pictures 01"); the
    // Vulkan gate's only label, "AAAA", has no space. The quad is left as it is (the
    // previous glyph's box, or the run's hull under a transform): a transparent source
    // changes nothing under any blend pipeline.
    builder.emit(abi::label(&blank));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_MISC + 12,
    ));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.item + ITEM_OFFSET_ARC + ITEM_ARC_GLYPH_HEIGHT,
    ));
    builder.emit(abi::branch(&publish));

    // plan-116-H: this function now PUBLISHES and does not draw. Its `vkCmdDraw` moved
    // to `emit_draw_list_pass`, with every other draw, because the draw list is in scene
    // order and a group's blocks are shared — neither of which a draw issued from the
    // publish walk can honour (H9).
    //
    // Leaving the draw here was measured, not theorised: the glyphs were then drawn
    // twice, once from here with the push constant still undefined for that command, so
    // the vertex stage offset them by whatever the range held. The Vulkan harness
    // reported the render disagreeing with the oracle on 35.1% of pixels, worst=255,
    // while every draw-list assertion passed — the list was right and the command
    // stream had extra draws in it.
    //
    // The block is still published one per glyph quad at the shared cursor, which is
    // what lets the draw list count a text run as N contiguous instances and issue it
    // as a single instanced call.

    builder.emit(abi::label(&next));
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_index,
    ));
    builder.emit(abi::add_immediate(abi::SCRATCH[0], abi::SCRATCH[0], 1));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        at.glyph_index,
    ));
    builder.emit(abi::branch(&head));
    builder.emit(abi::label(&done));
}

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

/// bug-765: branch to `target` when `bit` of the pass word parked at `off_pass` is set
/// (`when_set`) or clear. Clobbers `SCRATCH[0]` and `SCRATCH[1]`.
pub(super) fn emit_branch_on_pass(
    builder: &mut CodeBuilder,
    off_pass: usize,
    bit: usize,
    when_set: bool,
    target: &str,
) {
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_pass,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &bit.to_string(),
    ));
    builder.emit(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    if when_set {
        builder.emit(abi::branch_ne(target));
    } else {
        builder.emit(abi::branch_eq(target));
    }
}

/// bug-765: make sure the render pass a frame's LATER passes record under exists, in
/// the state word `state_slot`: the one colour attachment of every other pass here,
/// LOADED and stored, entered and left in `layout` — the layout the frame's first pass
/// left the image in. It differs from the pass the pipelines were built against only
/// in load operation and layouts, which compatibility does not look at, so nothing is
/// rebuilt. Built once, on the graphics thread, by the first frame of several passes.
#[allow(clippy::too_many_arguments)]
fn emit_load_render_pass(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    state_slot: usize,
    layout: &str,
    unavailable: &str,
) -> Result<(), String> {
    let have = builder.label("vk_load_pass_have");
    emit_state_load(builder, off_state, state_slot, abi::SCRATCH[0]);
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(&have));
    let off_attachment = builder.allocate_stack_object("vk_load_attachment", ATTACHMENT_SIZE + 4);
    let off_ref = builder.allocate_stack_object("vk_load_attachment_ref", ATTACHMENT_REF_SIZE);
    let off_subpass = builder.allocate_stack_object("vk_load_subpass", SUBPASS_SIZE);
    let off_info = builder.allocate_stack_object("vk_load_pass_info", RENDER_PASS_INFO_SIZE);
    emit_struct(
        builder,
        off_attachment,
        ATTACHMENT_SIZE + 4,
        &[
            (ATTACHMENT_FORMAT, Field::U32(FORMAT_B8G8R8A8_SRGB)),
            (ATTACHMENT_SAMPLES, Field::U32(SAMPLE_COUNT_1)),
            (ATTACHMENT_LOAD_OP, Field::U32(ATTACHMENT_LOAD_OP_LOAD)),
            (ATTACHMENT_STORE_OP, Field::U32(ATTACHMENT_STORE_OP_STORE)),
            (ATTACHMENT_INITIAL_LAYOUT, Field::U32(layout)),
            (ATTACHMENT_FINAL_LAYOUT, Field::U32(layout)),
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
        off_info,
        RENDER_PASS_INFO_SIZE,
        &[
            (RENDER_PASS_STYPE, Field::U32(ST_RENDER_PASS_CREATE_INFO)),
            (RENDER_PASS_ATTACHMENT_COUNT, Field::U32("1")),
            (RENDER_PASS_ATTACHMENTS, Field::Addr(off_attachment)),
            (RENDER_PASS_SUBPASS_COUNT, Field::U32("1")),
            (RENDER_PASS_SUBPASSES, Field::Addr(off_subpass)),
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
        off_info,
        off_out,
        state_slot,
        unavailable,
    )?;
    builder.emit(abi::label(&have));
    Ok(())
}

/// Park the graphics-state block's address in `slot`, once, so every later access is
/// a load rather than another `adrp`/`add` pair.
fn state_base_into(builder: &mut CodeBuilder, slot: usize) {
    let symbol = builder.current_symbol.clone();
    push_symbol_address(
        &symbol,
        GRAPHICS_STATE_SYMBOL,
        abi::SCRATCH[0],
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::store_u64(abi::SCRATCH[0], abi::stack_pointer(), slot));
}

fn emit_state_store(
    builder: &mut CodeBuilder,
    state_slot: usize,
    offset: usize,
    value: impl Into<abi::Reg>,
) {
    let value: abi::Reg = value.into();
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        state_slot,
    ));
    builder.emit(abi::store_u64(value, abi::SCRATCH[3], offset));
}

fn emit_state_load(
    builder: &mut CodeBuilder,
    state_slot: usize,
    offset: usize,
    dst: impl Into<abi::Reg>,
) {
    // No temporary at all: the state base goes into `dst` itself, which the second load
    // overwrites anyway. Both kinds of temporary have broken this helper on x86-64:
    //
    // * A fixed `SCRATCH[k]`. The x86 scratch pool aliases the C argument bank
    //   (`map_scratch_register`): `SCRATCH[3]` is `r8`, which is `c_arg(4)`. This helper
    //   is called *between* argument stagings, so a fixed scratch overwrote an argument
    //   already in place — `vkCmdBindDescriptorSets` received the graphics-state pointer
    //   as its `descriptorSetCount` and walked a one-element array for a dozen entries.
    // * A `builder.temporary_vreg()`. The allocator does not see the hand-assigned
    //   `SCRATCH` values the emitters keep live, so it may colour the vreg onto one of
    //   them: bug-688's band index held its table offset in `SCRATCH[5]` (`r10`) across
    //   this call, the vreg came out as `r10`, and the table pointer became the state
    //   block plus 4 MiB — an access violation on Windows, and nothing on AArch64, where
    //   `x9`–`x17` are outside the allocatable file.
    let dst: abi::Reg = dst.into();
    builder.emit(abi::load_u64(dst, abi::stack_pointer(), state_slot));
    builder.emit(abi::load_u64(dst, dst, offset));
}

/// Store `value` (a whole number in a GPR) as an IEEE-754 **single** at
/// `sp + offset`.
///
/// `VkViewport` is the one place this backend needs a real `float` — everything else
/// crosses as 16.16 fixed point or a whole number. The assembler has no double→single
/// convert and no 32-bit floating-point store, which is the same gap that made the
/// item block fixed point, so the bit pattern is assembled with integer arithmetic:
/// convert to a double (which the assembler *does* have), read its bits out with
/// `fmov`, then re-lay the sign, the rebiased exponent and the top 23 mantissa bits.
///
/// Exact for the values it is given. A viewport dimension is a positive integer well
/// under 2^24, so it is representable in `float` with no rounding at all and the
/// mantissa bits being dropped are all zero. It is **not** a general
/// double-to-float — it has no rounding, no denormals, and no zero case — which is
/// why it is spelled as a private helper for this one use rather than an `abi::`
/// primitive that would invite the general one.
fn emit_store_f32_from_integer(
    builder: &mut CodeBuilder,
    value: impl Into<abi::Reg>,
    offset: usize,
) {
    let bits = abi::SCRATCH[8];
    let sign = abi::SCRATCH[9];
    let exponent = abi::SCRATCH[10];
    let mantissa = abi::SCRATCH[11];

    builder.emit(abi::signed_convert_to_float_d(abi::FP_SCRATCH[7], value));
    builder.emit(abi::float_move_x_from_d(bits, abi::FP_SCRATCH[7]));

    // sign = bits >> 63, in place at bit 31.
    builder.emit(abi::shift_right_immediate(sign, bits, 63));
    builder.emit(abi::shift_left_immediate(sign, sign, 31));

    // exponent = ((bits >> 52) & 0x7FF) - 1023 + 127, at bit 23.
    builder.emit(abi::shift_right_immediate(exponent, bits, 52));
    builder.emit(abi::move_immediate(mantissa, "Integer", "2047"));
    builder.emit(abi::and_registers(exponent, exponent, mantissa));
    builder.emit(abi::subtract_immediate(exponent, exponent, 896));
    builder.emit(abi::shift_left_immediate(exponent, exponent, 23));

    // mantissa = (bits >> 29) & 0x7FFFFF — the top 23 of the double's 52.
    builder.emit(abi::shift_right_immediate(mantissa, bits, 29));
    builder.emit(abi::move_immediate(bits, "Integer", "8388607"));
    builder.emit(abi::and_registers(mantissa, mantissa, bits));

    builder.emit(abi::or_registers(sign, sign, exponent));
    builder.emit(abi::or_registers(sign, sign, mantissa));
    builder.emit(abi::store_u32(sign, abi::stack_pointer(), offset));
}

/// Call the function pointer parked at `off_fn`.
///
/// Through a **fresh vreg** — see the module doc. Never a fixed `abi::SCRATCH[k]`.
/// Stage a zero as the `n`-th integer C argument, wherever this target passes it.
///
/// **`c_arg(n)` is not "argument n" — it is slot n of the aligned call bank**, and
/// the bank is longer than the ABI's register argument list. On x86-64 SysV the bank
/// runs `[rdi, rsi, rdx, rcx, r8, r9, rax, rbp]` but only the first six carry
/// arguments, so `c_arg(6)` writes `rax` and `c_arg(7)` writes **`rbp`** — the frame
/// pointer — while the callee reads two stack slots nothing wrote. Staging into them
/// and spilling afterwards does not help: the damage is the write itself. AArch64 and
/// riscv64 pass eight in registers, so there the same indices are exactly right, which
/// is why this is invisible on the development host.
///
/// It cost a blank frame to find. `vkCmdBindDescriptorSets` is this backend's only
/// eight-argument call; validation reported a `descriptorSetCount` in the dozens and
/// a dozen "invalid VkDescriptorSet" handles read past the end of a one-element array,
/// all of it downstream of a zeroed `rbp`.
///
/// Routed through the register model rather than a hardcoded six, so on a target whose
/// registers cover `n` the emitted bytes are the plain `c_arg(n)` staging.
fn emit_int_arg_zero(builder: &mut CodeBuilder, platform: &dyn CodegenPlatform, n: usize) {
    emit_int_arg(builder, platform, n, "0");
}

/// Stage integer immediate `value` as C argument `n`.
///
/// **This is not optional past argument four on Win64.** `CALL_ARGS_WIN64` is
/// `[rcx, rdx, r8, r9, rdi, rsi, ...]`, so `c_arg(4)`/`c_arg(5)` realize to `rdi`/`rsi`
/// — registers Win64 does not pass arguments in at all. The 5th argument onward travels
/// on the stack above the shadow space, which is what `outgoing_stack_arg_store` writes.
/// SysV covers six in registers, so there the emitted bytes are the plain `c_arg(n)`
/// staging and nothing changes.
///
/// plan-98-F Phase 3 found this the hard way: `vkMapMemory` takes six arguments, and its
/// `ppData` out-param arrived in `rsi`, which lavapipe never reads — so the ICD wrote the
/// mapped pointer through whatever `ppData` happened to hold and faulted at
/// `vulkan_lvp!vk_icdGetInstanceProcAddr+0x818`, `mov [r8],rax` with `r8=8`.
fn emit_int_arg(builder: &mut CodeBuilder, platform: &dyn CodegenPlatform, n: usize, value: &str) {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::move_immediate(abi::c_arg(n), "Integer", value));
        return;
    }
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    builder.emit(abi::outgoing_stack_arg_store(
        abi::SCRATCH[0],
        n - register_args,
    ));
}

/// Stage the integer **stored at** stack offset `offset` as C argument `n`.
///
/// The load-from-memory counterpart of [`emit_int_arg`], and the reason it exists is
/// the hazard in that function's own doc turned around: an argument whose value is
/// computed rather than constant cannot wait in a scratch register while the other
/// arguments are staged, because on x86-64 the scratch pool aliases the argument bank.
/// Parking it on the stack and loading it straight into its argument register makes
/// the staging order irrelevant. Same register-model rule as [`emit_int_arg`].
fn emit_int_arg_slot(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    n: usize,
    offset: usize,
) {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::load_u64(abi::c_arg(n), abi::stack_pointer(), offset));
        return;
    }
    builder.emit(abi::load_u64(abi::SCRATCH[0], abi::stack_pointer(), offset));
    builder.emit(abi::outgoing_stack_arg_store(
        abi::SCRATCH[0],
        n - register_args,
    ));
}

/// Stage the address of the stack object at `offset` as C argument `n` — the
/// `&out` / `&structure` shape every Vulkan out-parameter uses. Same register-model
/// rule as [`emit_int_arg`].
fn emit_addr_arg(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    n: usize,
    offset: usize,
) {
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::add_immediate(
            abi::c_arg(n),
            abi::stack_pointer(),
            offset,
        ));
        return;
    }
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        offset,
    ));
    builder.emit(abi::outgoing_stack_arg_store(
        abi::SCRATCH[0],
        n - register_args,
    ));
}

fn emit_call_fn(builder: &mut CodeBuilder, off_fn: usize) {
    let callee = builder.temporary_vreg();
    builder.emit(abi::load_u64(&callee, abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(&callee));
}

/// `fn = dlsym(handle, "<name>")` into `off_fn`; branch away if null.
///
/// Every entry point is resolved from the loader library directly rather than
/// bootstrapped through `vkGetInstanceProcAddr`. The spec only *guarantees* that
/// `vkGetInstanceProcAddr` is exported, so the bootstrap is the textbook route — but
/// measured on box 2228 (loader 1.4.309), `vkGetInstanceProcAddr(NULL,
/// "vkCreateInstance")` returned NULL where `dlsym(handle, "vkCreateInstance")`
/// returned a working pointer, and `libvulkan.so.1` exports every core entry point by
/// name.
///
/// It is also the simpler shape: `dlsym` goes through `emit_external_call`, the path
/// the `audio` backend has proven on both libcs, instead of a hand-staged call
/// through a resolved pointer. The one thing this must not become is a silent
/// fallback — a name that is genuinely absent still returns FALSE from the probe,
/// which is what "no Vulkan here" is supposed to mean.
#[allow(clippy::too_many_arguments)]
/// Open the Vulkan loader library, leaving the handle in `c_return(0)` (0 on failure).
///
/// POSIX takes `dlopen(name, RTLD_NOW)`. Windows takes `LoadLibraryExA(name, NULL, 0)`:
/// a three-argument call, `hFile` reserved and required to be NULL, and flags 0 for the
/// **default search order** — this is a system library found in `System32`, not one of
/// the exe-relative `vendor/` DLLs that `emit_lib_open` builds an absolute path for.
///
/// Deliberately NOT routed through the `emit_lib_open` platform hook the `LINK` loader
/// uses. That hook answers in `return_register()`, the aligned MFB bank, while every
/// check and store in this backend reads `c_return(0)` — and those are *different
/// registers on Win64* (`rcx` vs `rax`) and the same one on AArch64, which is exactly
/// the class of defect bug-478/479 spent eight fixes on. Answering in `c_return(0)`
/// here keeps this file's single convention.
fn emit_open_library(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    let windows = matches!(platform.family(), PlatformFamily::Windows);
    emit_data_address(
        &symbol,
        abi::c_arg(0),
        &soname_symbol(),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    if windows {
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0")); // hFile, reserved
        builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0")); // default search
    } else {
        builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    }
    platform.emit_external_call(
        if windows { "LoadLibraryExA" } else { "dlopen" },
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )
}

fn emit_dlsym(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    name: &str,
    off_handle: usize,
    off_fn: usize,
    unavailable: &str,
) -> Result<(), String> {
    let symbol = builder.current_symbol.clone();
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_handle,
    ));
    emit_data_address(
        &symbol,
        abi::c_arg(1),
        &symbol_name_symbol(name),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    // `GetProcAddress(hModule, lpProcName)` is `dlsym`'s exact shape — two arguments in
    // the same order, the address in the C result, NULL when the name is absent — so
    // only the callee name changes.
    platform.emit_external_call(
        if matches!(platform.family(), PlatformFamily::Windows) {
            "GetProcAddress"
        } else {
            "dlsym"
        },
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(unavailable));
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_fn,
    ));
    Ok(())
}

/// Zero `length` bytes at `sp + base`, eight at a time.
///
/// Vulkan's create-info structs are mostly zero, and a *reserved* field left holding
/// stack garbage is not ignored — `flags` and `pNext` in particular are read. Zeroing
/// the whole range and then writing only the named members is what makes the field
/// tables above complete rather than merely representative.
fn emit_zero_range(builder: &mut CodeBuilder, base: usize, length: usize) {
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    for offset in (base..base + length).step_by(8) {
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            offset,
        ));
    }
}

/// Check one hand-transcribed C layout: every `(member, offset, width)` lies inside
/// the struct at its natural alignment, no two members overlap, and the size is a
/// multiple of the struct's alignment. What it cannot check is that the offsets match
/// the Vulkan headers — only a driver can — but a transcription slip (a member moved,
/// a width misread, a pointer at a 4-byte offset) almost always breaks one of these.
#[cfg(test)]
fn assert_layout(name: &str, size: usize, align: usize, members: &[(&str, usize, usize)]) {
    assert_eq!(
        size % align,
        0,
        "{name}: size {size} is not a multiple of {align}"
    );
    let mut sorted = members.to_vec();
    sorted.sort_by_key(|&(_, offset, _)| offset);
    let mut end = 0;
    for (member, offset, width) in sorted {
        assert!(
            offset + width <= size,
            "{name}.{member}: [{offset}, {}) runs past the {size}-byte struct",
            offset + width
        );
        assert_eq!(
            offset % width.min(8),
            0,
            "{name}.{member}: offset {offset} is not {}-byte aligned",
            width.min(8)
        );
        assert!(
            offset >= end,
            "{name}.{member}: offset {offset} overlaps the member ending at {end}"
        );
        end = offset + width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::runtime::canvas::{
        GRAPHICS_OFFSET_MTL_PIPELINE_MODES, GRAPHICS_STATE_SIZE,
    };
    use crate::codegen::runtime::canvas::{VULKAN_EDGE_BYTES, VULKAN_EDGE_RESIDUAL_BYTES};

    /// Every struct table above, member by member with its C width — the check the
    /// section header promises.
    #[test]
    fn layout_tables_are_self_consistent() {
        assert_layout(
            "VkApplicationInfo",
            APP_INFO_SIZE,
            8,
            &[
                ("sType", APP_INFO_STYPE, 4),
                ("apiVersion", APP_INFO_API_VERSION, 4),
            ],
        );
        assert_layout(
            "VkInstanceCreateInfo",
            INSTANCE_INFO_SIZE,
            8,
            &[
                ("sType", INSTANCE_INFO_STYPE, 4),
                ("pApplicationInfo", INSTANCE_INFO_APP_INFO, 8),
            ],
        );
        assert_layout(
            "VkDeviceQueueCreateInfo",
            QUEUE_INFO_SIZE,
            8,
            &[
                ("sType", QUEUE_INFO_STYPE, 4),
                ("queueFamilyIndex", QUEUE_INFO_FAMILY, 4),
                ("queueCount", QUEUE_INFO_COUNT, 4),
                ("pQueuePriorities", QUEUE_INFO_PRIORITIES, 8),
            ],
        );
        assert_layout(
            "VkDeviceCreateInfo",
            DEVICE_INFO_SIZE,
            8,
            &[
                ("sType", DEVICE_INFO_STYPE, 4),
                ("queueCreateInfoCount", DEVICE_INFO_QUEUE_COUNT, 4),
                ("pQueueCreateInfos", DEVICE_INFO_QUEUE_INFOS, 8),
            ],
        );
        assert_layout(
            "VkQueueFamilyProperties",
            QUEUE_FAMILY_PROPERTIES_SIZE,
            4,
            &[("queueFlags", QUEUE_FAMILY_FLAGS, 4)],
        );
        assert_layout(
            "VkShaderModuleCreateInfo",
            SHADER_MODULE_INFO_SIZE,
            8,
            &[
                ("sType", SHADER_MODULE_STYPE, 4),
                ("codeSize", SHADER_MODULE_CODE_SIZE, 8),
                ("pCode", SHADER_MODULE_CODE, 8),
            ],
        );
        assert_layout(
            "VkPipelineLayoutCreateInfo",
            LAYOUT_INFO_SIZE,
            8,
            &[
                ("sType", LAYOUT_INFO_STYPE, 4),
                ("setLayoutCount", LAYOUT_INFO_SET_COUNT, 4),
                ("pSetLayouts", LAYOUT_INFO_SETS, 8),
                ("pushConstantRangeCount", LAYOUT_INFO_RANGE_COUNT, 4),
                ("pPushConstantRanges", LAYOUT_INFO_RANGES, 8),
            ],
        );
        assert_layout(
            "VkPushConstantRange",
            PUSH_RANGE_SIZE,
            4,
            &[
                ("stageFlags", PUSH_RANGE_STAGES, 4),
                ("offset", PUSH_RANGE_OFFSET, 4),
                ("size", PUSH_RANGE_BYTES, 4),
            ],
        );
        assert_layout(
            "VkAttachmentDescription",
            ATTACHMENT_SIZE,
            4,
            &[
                ("format", ATTACHMENT_FORMAT, 4),
                ("samples", ATTACHMENT_SAMPLES, 4),
                ("loadOp", ATTACHMENT_LOAD_OP, 4),
                ("storeOp", ATTACHMENT_STORE_OP, 4),
                ("initialLayout", ATTACHMENT_INITIAL_LAYOUT, 4),
                ("finalLayout", ATTACHMENT_FINAL_LAYOUT, 4),
            ],
        );
        assert_layout(
            "VkAttachmentReference",
            ATTACHMENT_REF_SIZE,
            4,
            &[
                ("attachment", ATTACHMENT_REF_INDEX, 4),
                ("layout", ATTACHMENT_REF_LAYOUT, 4),
            ],
        );
        assert_layout(
            "VkSubpassDescription",
            SUBPASS_SIZE,
            8,
            &[
                ("pipelineBindPoint", SUBPASS_BIND_POINT, 4),
                ("colorAttachmentCount", SUBPASS_COLOR_COUNT, 4),
                ("pColorAttachments", SUBPASS_COLOR_ATTACHMENTS, 8),
            ],
        );
        assert_layout(
            "VkRenderPassCreateInfo",
            RENDER_PASS_INFO_SIZE,
            8,
            &[
                ("sType", RENDER_PASS_STYPE, 4),
                ("attachmentCount", RENDER_PASS_ATTACHMENT_COUNT, 4),
                ("pAttachments", RENDER_PASS_ATTACHMENTS, 8),
                ("subpassCount", RENDER_PASS_SUBPASS_COUNT, 4),
                ("pSubpasses", RENDER_PASS_SUBPASSES, 8),
            ],
        );
        assert_layout(
            "VkPipelineShaderStageCreateInfo",
            STAGE_INFO_SIZE,
            8,
            &[
                ("sType", STAGE_INFO_STYPE, 4),
                ("stage", STAGE_INFO_STAGE, 4),
                ("module", STAGE_INFO_MODULE, 8),
                ("pName", STAGE_INFO_NAME, 8),
                ("pSpecializationInfo", STAGE_INFO_SPECIALIZATION, 8),
            ],
        );
        assert_layout(
            "VkSpecializationInfo",
            SPEC_INFO_SIZE,
            8,
            &[
                ("mapEntryCount", SPEC_INFO_COUNT, 4),
                ("pMapEntries", SPEC_INFO_ENTRIES, 8),
                ("dataSize", SPEC_INFO_DATA_SIZE, 8),
                ("pData", SPEC_INFO_DATA, 8),
            ],
        );
        assert_layout(
            "VkSpecializationMapEntry",
            SPEC_ENTRY_SIZE,
            8,
            &[
                ("constantID", SPEC_ENTRY_ID, 4),
                ("offset", SPEC_ENTRY_OFFSET, 4),
                ("size", SPEC_ENTRY_BYTES, 8),
            ],
        );
        assert_layout(
            "VkPipelineVertexInputStateCreateInfo",
            VERTEX_INPUT_INFO_SIZE,
            8,
            &[],
        );
        assert_layout(
            "VkPipelineInputAssemblyStateCreateInfo",
            INPUT_ASSEMBLY_INFO_SIZE,
            8,
            &[("topology", INPUT_ASSEMBLY_TOPOLOGY, 4)],
        );
        assert_layout(
            "VkPipelineViewportStateCreateInfo",
            VIEWPORT_INFO_SIZE,
            8,
            &[
                ("viewportCount", VIEWPORT_INFO_VIEWPORT_COUNT, 4),
                ("scissorCount", VIEWPORT_INFO_SCISSOR_COUNT, 4),
            ],
        );
        assert_layout(
            "VkPipelineRasterizationStateCreateInfo",
            RASTER_INFO_SIZE,
            8,
            &[
                ("polygonMode", RASTER_POLYGON_MODE, 4),
                ("cullMode", RASTER_CULL_MODE, 4),
                ("frontFace", RASTER_FRONT_FACE, 4),
                ("lineWidth", RASTER_LINE_WIDTH, 4),
            ],
        );
        assert_layout(
            "VkPipelineMultisampleStateCreateInfo",
            MULTISAMPLE_INFO_SIZE,
            8,
            &[("rasterizationSamples", MULTISAMPLE_SAMPLES, 4)],
        );
        assert_layout(
            "VkPipelineColorBlendAttachmentState",
            BLEND_ATTACHMENT_SIZE,
            4,
            &[
                ("blendEnable", BLEND_ENABLE, 4),
                ("srcColorBlendFactor", BLEND_SRC_COLOR, 4),
                ("dstColorBlendFactor", BLEND_DST_COLOR, 4),
                ("colorBlendOp", BLEND_COLOR_OP, 4),
                ("srcAlphaBlendFactor", BLEND_SRC_ALPHA, 4),
                ("dstAlphaBlendFactor", BLEND_DST_ALPHA, 4),
                ("alphaBlendOp", BLEND_ALPHA_OP, 4),
                ("colorWriteMask", BLEND_WRITE_MASK, 4),
            ],
        );
        assert_layout(
            "VkPipelineColorBlendStateCreateInfo",
            BLEND_INFO_SIZE,
            8,
            &[
                ("attachmentCount", BLEND_INFO_ATTACHMENT_COUNT, 4),
                ("pAttachments", BLEND_INFO_ATTACHMENTS, 8),
            ],
        );
        assert_layout(
            "VkPipelineDynamicStateCreateInfo",
            DYNAMIC_INFO_SIZE,
            8,
            &[
                ("dynamicStateCount", DYNAMIC_INFO_COUNT, 4),
                ("pDynamicStates", DYNAMIC_INFO_STATES, 8),
            ],
        );
        assert_layout(
            "VkGraphicsPipelineCreateInfo",
            PIPELINE_INFO_SIZE,
            8,
            &[
                ("stageCount", PIPELINE_STAGE_COUNT, 4),
                ("pStages", PIPELINE_STAGES, 8),
                ("pVertexInputState", PIPELINE_VERTEX_INPUT, 8),
                ("pInputAssemblyState", PIPELINE_INPUT_ASSEMBLY, 8),
                ("pViewportState", PIPELINE_VIEWPORT, 8),
                ("pRasterizationState", PIPELINE_RASTER, 8),
                ("pMultisampleState", PIPELINE_MULTISAMPLE, 8),
                ("pColorBlendState", PIPELINE_COLOR_BLEND, 8),
                ("pDynamicState", PIPELINE_DYNAMIC, 8),
                ("layout", PIPELINE_LAYOUT, 8),
                ("renderPass", PIPELINE_RENDER_PASS, 8),
            ],
        );
        assert_layout(
            "VkImageCreateInfo",
            IMAGE_INFO_SIZE,
            8,
            &[
                ("imageType", IMAGE_INFO_TYPE, 4),
                ("format", IMAGE_INFO_FORMAT, 4),
                ("extent.width", IMAGE_INFO_WIDTH, 4),
                ("extent.height", IMAGE_INFO_HEIGHT, 4),
                ("extent.depth", IMAGE_INFO_DEPTH, 4),
                ("mipLevels", IMAGE_INFO_MIP_LEVELS, 4),
                ("arrayLayers", IMAGE_INFO_ARRAY_LAYERS, 4),
                ("samples", IMAGE_INFO_SAMPLES, 4),
                ("tiling", IMAGE_INFO_TILING, 4),
                ("usage", IMAGE_INFO_USAGE, 4),
                ("initialLayout", IMAGE_INFO_INITIAL_LAYOUT, 4),
            ],
        );
        assert_layout(
            "VkMemoryRequirements",
            MEMORY_REQS_SIZE,
            8,
            &[
                ("size", MEMORY_REQS_BYTES, 8),
                ("memoryTypeBits", MEMORY_REQS_TYPE_BITS, 4),
            ],
        );
        assert_layout(
            "VkMemoryAllocateInfo",
            ALLOC_INFO_SIZE,
            8,
            &[
                ("allocationSize", ALLOC_INFO_BYTES, 8),
                ("memoryTypeIndex", ALLOC_INFO_TYPE_INDEX, 4),
            ],
        );
        assert_layout(
            "VkPhysicalDeviceMemoryProperties",
            MEMORY_PROPERTIES_SIZE,
            8,
            &[
                ("memoryTypeCount", MEMORY_PROPERTIES_TYPE_COUNT, 4),
                // `VkMemoryType[32]`, two `u32`s each: its first and last members.
                ("memoryTypes[0].propertyFlags", MEMORY_PROPERTIES_TYPES, 4),
                (
                    "memoryTypes[31].heapIndex",
                    MEMORY_PROPERTIES_TYPES + 31 * MEMORY_TYPE_STRIDE + 4,
                    4,
                ),
            ],
        );
        assert_layout(
            "VkImageViewCreateInfo",
            VIEW_INFO_SIZE,
            8,
            &[
                ("image", VIEW_INFO_IMAGE, 8),
                ("viewType", VIEW_INFO_TYPE, 4),
                ("format", VIEW_INFO_FORMAT, 4),
                ("subresourceRange.aspectMask", VIEW_INFO_ASPECT, 4),
                ("subresourceRange.levelCount", VIEW_INFO_LEVEL_COUNT, 4),
                ("subresourceRange.layerCount", VIEW_INFO_LAYER_COUNT, 4),
            ],
        );
        assert_layout(
            "VkFramebufferCreateInfo",
            FRAMEBUFFER_INFO_SIZE,
            8,
            &[
                ("renderPass", FRAMEBUFFER_RENDER_PASS, 8),
                ("attachmentCount", FRAMEBUFFER_ATTACHMENT_COUNT, 4),
                ("pAttachments", FRAMEBUFFER_ATTACHMENTS, 8),
                ("width", FRAMEBUFFER_WIDTH, 4),
                ("height", FRAMEBUFFER_HEIGHT, 4),
                ("layers", FRAMEBUFFER_LAYERS, 4),
            ],
        );
        assert_layout(
            "VkBufferCreateInfo",
            BUFFER_INFO_SIZE,
            8,
            &[
                ("size", BUFFER_INFO_BYTES, 8),
                ("usage", BUFFER_INFO_USAGE, 4),
            ],
        );
        assert_layout(
            "VkCommandPoolCreateInfo",
            POOL_INFO_SIZE,
            8,
            &[
                ("flags", POOL_INFO_FLAGS, 4),
                ("queueFamilyIndex", POOL_INFO_FAMILY, 4),
            ],
        );
        assert_layout(
            "VkCommandBufferAllocateInfo",
            CMD_ALLOC_INFO_SIZE,
            8,
            &[
                ("commandPool", CMD_ALLOC_POOL, 8),
                ("level", CMD_ALLOC_LEVEL, 4),
                ("commandBufferCount", CMD_ALLOC_COUNT, 4),
            ],
        );
        assert_layout(
            "VkCommandBufferBeginInfo",
            CMD_BEGIN_INFO_SIZE,
            8,
            &[("flags", CMD_BEGIN_FLAGS, 4)],
        );
        assert_layout(
            "VkRenderPassBeginInfo",
            PASS_BEGIN_INFO_SIZE,
            8,
            &[
                ("renderPass", PASS_BEGIN_RENDER_PASS, 8),
                ("framebuffer", PASS_BEGIN_FRAMEBUFFER, 8),
                ("renderArea.extent.width", PASS_BEGIN_AREA_WIDTH, 4),
                ("renderArea.extent.height", PASS_BEGIN_AREA_HEIGHT, 4),
                ("clearValueCount", PASS_BEGIN_CLEAR_COUNT, 4),
                ("pClearValues", PASS_BEGIN_CLEARS, 8),
            ],
        );
        assert_layout(
            "VkClearValue",
            CLEAR_VALUE_SIZE,
            4,
            &[("color.float32[3]", CLEAR_ALPHA, 4)],
        );
        assert_layout(
            "VkViewport",
            VIEWPORT_SIZE,
            4,
            &[
                ("width", VIEWPORT_WIDTH, 4),
                ("height", VIEWPORT_HEIGHT, 4),
                ("maxDepth", VIEWPORT_MAX_DEPTH, 4),
            ],
        );
        assert_layout(
            "VkRect2D",
            RECT_SIZE,
            4,
            &[
                ("extent.width", RECT_WIDTH, 4),
                ("extent.height", RECT_HEIGHT, 4),
            ],
        );
        assert_layout(
            "VkBufferImageCopy",
            COPY_SIZE,
            8,
            &[
                ("imageSubresource.aspectMask", COPY_ASPECT, 4),
                ("imageSubresource.layerCount", COPY_LAYER_COUNT, 4),
                ("imageExtent.width", COPY_EXTENT_WIDTH, 4),
                ("imageExtent.height", COPY_EXTENT_HEIGHT, 4),
                ("imageExtent.depth", COPY_EXTENT_DEPTH, 4),
            ],
        );
        assert_layout(
            "VkSubmitInfo",
            SUBMIT_INFO_SIZE,
            8,
            &[
                ("commandBufferCount", SUBMIT_COMMAND_COUNT, 4),
                ("pCommandBuffers", SUBMIT_COMMANDS, 8),
            ],
        );
        assert_layout(
            "VkDescriptorSetLayoutBinding",
            BINDING_SIZE,
            8,
            &[
                ("binding", BINDING_INDEX, 4),
                ("descriptorType", BINDING_TYPE, 4),
                ("descriptorCount", BINDING_COUNT, 4),
                ("stageFlags", BINDING_STAGES, 4),
            ],
        );
        assert_layout(
            "VkDescriptorSetLayoutCreateInfo",
            SET_LAYOUT_INFO_SIZE,
            8,
            &[
                ("bindingCount", SET_LAYOUT_BINDING_COUNT, 4),
                ("pBindings", SET_LAYOUT_BINDINGS, 8),
            ],
        );
        assert_layout(
            "VkDescriptorPoolSize",
            POOL_SIZE_SIZE,
            4,
            &[
                ("type", POOL_SIZE_TYPE, 4),
                ("descriptorCount", POOL_SIZE_COUNT, 4),
            ],
        );
        assert_layout(
            "VkDescriptorPoolCreateInfo",
            DESC_POOL_INFO_SIZE,
            8,
            &[
                ("maxSets", DESC_POOL_MAX_SETS, 4),
                ("poolSizeCount", DESC_POOL_SIZE_COUNT, 4),
                ("pPoolSizes", DESC_POOL_SIZES, 8),
            ],
        );
        assert_layout(
            "VkDescriptorSetAllocateInfo",
            DESC_ALLOC_INFO_SIZE,
            8,
            &[
                ("descriptorPool", DESC_ALLOC_POOL, 8),
                ("descriptorSetCount", DESC_ALLOC_SET_COUNT, 4),
                ("pSetLayouts", DESC_ALLOC_LAYOUTS, 8),
            ],
        );
        assert_layout(
            "VkDescriptorBufferInfo",
            DESC_BUFFER_INFO_SIZE,
            8,
            &[
                ("buffer", DESC_BUFFER_INFO_BUFFER, 8),
                ("range", DESC_BUFFER_INFO_RANGE, 8),
            ],
        );
        assert_layout(
            "VkWriteDescriptorSet",
            WRITE_SET_SIZE,
            8,
            &[
                ("dstSet", WRITE_SET_DST, 8),
                ("dstBinding", WRITE_SET_BINDING, 4),
                ("descriptorCount", WRITE_SET_COUNT, 4),
                ("descriptorType", WRITE_SET_TYPE, 4),
                ("pBufferInfo", WRITE_SET_BUFFER_INFO, 8),
            ],
        );
    }

    /// bug-688 V5: the readback buffer is read by the CPU every frame, so it asks for
    /// a `HOST_CACHED` type first and only then the plain coherent one. On box 206's
    /// Intel UHD 630 the first coherent type is write-combined (uncached reads) and
    /// took ~200 ms a frame to read back; the cached type the same driver offers cut
    /// the 95-item frame's render phase to ~13 ms.
    #[test]
    fn the_readback_buffer_prefers_cached_memory() {
        let flags: Vec<u32> = READBACK_MEMORY_CANDIDATES
            .iter()
            .map(|f| f.parse().expect("numeric flags"))
            .collect();
        const HOST_VISIBLE: u32 = 0x2;
        const HOST_COHERENT: u32 = 0x4;
        const HOST_CACHED: u32 = 0x8;
        assert_eq!(
            flags[0],
            HOST_VISIBLE | HOST_COHERENT | HOST_CACHED,
            "the first candidate must be the cached, coherent, host-visible type"
        );
        assert_eq!(
            *flags.last().unwrap(),
            HOST_VISIBLE | HOST_COHERENT,
            "the last candidate must be the coherent type every device offers"
        );
        assert!(
            flags
                .iter()
                .all(|f| f & (HOST_VISIBLE | HOST_COHERENT) == HOST_VISIBLE | HOST_COHERENT),
            "every candidate must stay coherent: the copy is read with no invalidate"
        );
    }

    /// The embedded SPIR-V is well-formed.
    ///
    /// The blobs are checked-in binary, so the ways they go wrong are silent: a
    /// truncated copy, a text-mode transfer that mangles bytes, a regeneration that
    /// wrote the GLSL over the `.spv`. None of those fail the build — they fail
    /// inside `vkCreateShaderModule` on a machine the developer may not have. The
    /// magic word and word-alignment catch every one of them here.
    #[test]
    fn the_embedded_spirv_is_well_formed() {
        for (name, blob) in [("vertex", SPIRV_VERTEX), ("fragment", SPIRV_FRAGMENT)] {
            assert!(
                blob.len() >= 20,
                "the {name} SPIR-V is {} bytes — too short to hold a header",
                blob.len()
            );
            assert_eq!(
                blob.len() % 4,
                0,
                "the {name} SPIR-V is {} bytes, not a whole number of 32-bit words",
                blob.len()
            );
            let magic = u32::from_le_bytes([blob[0], blob[1], blob[2], blob[3]]);
            assert_eq!(
                magic, SPIRV_MAGIC,
                "the {name} blob does not start with SPIR-V's magic word; got {magic:#010x}"
            );
        }
    }

    /// The two backends' pipeline arrays hold one entry per `BlendMode`, and the two
    /// arrays do not overlap each other or run past the state block.
    ///
    /// The blend tag reaches the frame path from `HEADER_BLEND` as a plain 0..3 and is
    /// used as an *index* with no bounds check — `base + mode * 8` — so an array
    /// shorter than the variant set reads a neighbouring state slot and binds it as a
    /// pipeline handle. Nothing catches that at run time: the value is a pointer either
    /// way, and the frame comes back wrong rather than failing.
    #[test]
    fn each_backend_has_one_pipeline_slot_per_blend_mode() {
        assert_eq!(
            BLEND_MODE_COUNT, 4,
            "BlendMode has four variants (Normal, Multiply, Screen, Add); both pipeline \
             arrays are sized from this"
        );
        assert_eq!(
            GRAPHICS_OFFSET_MTL_PIPELINE_MODES,
            GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES + BLEND_MODE_COUNT * 8,
            "the Metal array must start where the Vulkan one ends: a gap wastes state, \
             and an overlap makes one backend bind the other's handles"
        );
        assert!(
            GRAPHICS_OFFSET_MTL_PIPELINE_MODES + BLEND_MODE_COUNT * 8 <= GRAPHICS_STATE_SIZE,
            "the pipeline arrays run past the end of the graphics state block"
        );
    }

    /// plan-172-J: the pointer's five words are 40 contiguous bytes, the sequence word
    /// first and the four published words after it in the order
    /// `canvas::pointerSnapshot` returns them. Three platform handlers and one reader
    /// address them by constant.
    ///
    /// plan-172-K put two words after them — `AUTOPAINT_LIVE`, then `AUTO_WAKE` — so
    /// the pointer's are no longer the block's last; the block is sized from the new
    /// last word, and that is what the first two assertions now say. Written as "the
    /// pointer words end the block" (plan-172-J), which was true of the block then and
    /// was never what the handlers depend on: they depend on the five being contiguous
    /// and in order, which the loop still checks.
    #[test]
    fn the_pointer_words_end_the_graphics_state() {
        use crate::codegen::runtime::canvas::{
            GRAPHICS_OFFSET_AUTOPAINT_LIVE, GRAPHICS_OFFSET_AUTO_WAKE,
            GRAPHICS_OFFSET_POINTER_INSIDE, GRAPHICS_OFFSET_POINTER_SEQ, GRAPHICS_POINTER_WORDS,
        };
        assert_eq!(
            GRAPHICS_OFFSET_AUTOPAINT_LIVE,
            GRAPHICS_OFFSET_POINTER_SEQ + 40
        );
        assert_eq!(
            GRAPHICS_OFFSET_POINTER_INSIDE + 8,
            GRAPHICS_OFFSET_AUTOPAINT_LIVE
        );
        assert_eq!(
            GRAPHICS_OFFSET_AUTO_WAKE,
            GRAPHICS_OFFSET_AUTOPAINT_LIVE + 8
        );
        // And the baked `options.canvasAutoPaintMS`, the block's last word and the only
        // one that does not start as zero.
        use crate::codegen::runtime::canvas::{
            graphics_state_data_object, GRAPHICS_OFFSET_AUTO_WINDOW_MS,
        };
        assert_eq!(
            GRAPHICS_OFFSET_AUTO_WINDOW_MS,
            GRAPHICS_OFFSET_AUTO_WAKE + 8
        );
        assert_eq!(GRAPHICS_STATE_SIZE, GRAPHICS_OFFSET_AUTO_WINDOW_MS + 8);
        let block = graphics_state_data_object(0x0102);
        assert_eq!(block.value.len(), GRAPHICS_STATE_SIZE * 2);
        let at = GRAPHICS_OFFSET_AUTO_WINDOW_MS * 2;
        assert_eq!(&block.value[at..at + 16], "0201000000000000");
        assert!(block.value[..at].bytes().all(|digit| digit == b'0'));
        for (index, offset) in GRAPHICS_POINTER_WORDS.into_iter().enumerate() {
            assert_eq!(offset, GRAPHICS_OFFSET_POINTER_SEQ + 8 * (index + 1));
        }
        assert_eq!(GRAPHICS_OFFSET_POINTER_SEQ % 8, 0);
    }

    /// The item block is the size the GLSL `ItemBlock`'s std430 array stride is.
    ///
    /// **This replaced a check against Vulkan's guaranteed 128-byte push-constant
    /// range** (plan-116-A). That bound was real while the block *was* a push constant:
    /// 128 is the `maxPushConstantsSize` minimum every implementation must support, and
    /// exceeding it would have failed at pipeline-layout creation on whichever machine
    /// had the smallest limit — which is exactly the machine the developer does not
    /// have. The block now rides a storage buffer, so that ceiling is gone, and
    /// asserting it would pin a constraint nothing enforces.
    ///
    /// What replaces it is the constraint that *is* still live: the CPU emitter writes
    /// records of `ITEM_BLOCK_SIZE` bytes and the shaders index an `ItemBlock[]`, so the
    /// two agree only if std430's array stride equals that size. std430 gives a struct
    /// the alignment of its largest member — `ivec4`, 16 bytes — and rounds the stride
    /// up to a multiple of it. So the stride equals the size exactly when the size is a
    /// multiple of 16, which is what this asserts, and it is why every member of the
    /// block is an `ivec4` rather than packed.
    ///
    /// Measured against the real compiler rather than reasoned about alone:
    /// `glslangValidator -V -q mfb_canvas.vert` reports `topLevelArrayStride 160` with
    /// members at 0/16/32/48/64/80/96/112/128/144 (2026-09-01, glslang 11:15.2.0) —
    /// re-measured each time the block grew: 112 → 128 for plan-116-B's clip, then
    /// 128 → 160 for plan-116-C's transform. A later letter that widens it again must
    /// re-run that and keep this equality.
    #[test]
    fn the_item_block_matches_the_std430_stride() {
        assert_eq!(
            ITEM_BLOCK_SIZE % 16,
            0,
            "the block is a run of ivec4s: std430 rounds its array stride up to a \
             multiple of 16, so a size that is not one makes the stride differ from the \
             {ITEM_BLOCK_SIZE} bytes the emitter writes per record, and every item after \
             the first would read a shifted block"
        );
        assert_eq!(
            VULKAN_ITEM_BUFFER_BYTES,
            VULKAN_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE,
            "the buffer must hold exactly the number of records the predicates admit"
        );
    }

    /// The fragment shader's region bases are specialization constants, and the
    /// pipeline feeds every one of them (bug-688).
    ///
    /// This replaced two tests that compared literals in the GLSL (`const int GLYPH_BASE
    /// = 65536;`, `const int GRADIENT_BASE = 1114112;`) against the Rust layout. Those
    /// literals are gone: each base is now `layout(constant_id = N) const int` with a
    /// default of 0, and its value arrives through `VkSpecializationInfo` from
    /// `VULKAN_SPEC_CONSTANTS`, which is built from the layout constants themselves. So
    /// what can still disagree is the NAME-to-ID pairing — the GLSL says `GLYPH_BASE` is
    /// id 0 and the Rust table must send the glyph base as id 0 — and whether the blob
    /// was regenerated from this GLSL at all. A wrong pairing, or a stale blob with the
    /// literals, does not fail anywhere: every glyph, ramp or band reads from another
    /// region of a buffer that is entirely valid memory.
    #[test]
    fn the_pipeline_specializes_every_region_base() {
        const GLSL: &str = include_str!("shaders/mfb_canvas.frag");
        let expected = [
            ("GLYPH_BASE", VULKAN_GLYPH_BASE_WORDS),
            ("GRADIENT_BASE", VULKAN_GRADIENT_BASE_WORDS),
            ("BAND_BASE", VULKAN_BAND_BASE_WORDS),
            // bug-687: the polygon vertices' residual words.
            ("EDGE_RESIDUAL_BASE", VULKAN_EDGE_RESIDUAL_BASE_WORDS),
            // plan-163-B: the dash words.
            ("DASH_BASE", VULKAN_DASH_BASE_WORDS),
        ];
        assert_eq!(
            VULKAN_SPEC_CONSTANTS.len(),
            expected.len(),
            "one specialization entry per region base the shader reads"
        );
        for (name, words) in expected {
            let line = GLSL
                .lines()
                .find(|l| l.contains(&format!("const int {name} ")))
                .unwrap_or_else(|| panic!("the fragment shader does not declare {name}"));
            let id: u32 = line
                .split("constant_id")
                .nth(1)
                .and_then(|rest| rest.trim_start_matches([' ', '=']).split(')').next())
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or_else(|| {
                    panic!("{name} is not a specialization constant in the GLSL: `{line}`")
                });
            assert!(
                VULKAN_SPEC_CONSTANTS.contains(&(id, words)),
                "the GLSL reads {name} as constant_id {id}, but the pipeline does not send \
                 its base ({words} words) under that id: {VULKAN_SPEC_CONSTANTS:?}"
            );
        }
        // The blob is compiled from that GLSL: each id the shader reads is decorated
        // `SpecId` (OpDecorate = 71, SpecId = 1) in the checked-in SPIR-V. A blob still
        // holding the literals has no such decoration.
        let words: Vec<u32> = SPIRV_FRAGMENT
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let mut spec_ids = Vec::new();
        let mut at = 5;
        while at < words.len() {
            let (count, opcode) = ((words[at] >> 16) as usize, words[at] & 0xffff);
            if opcode == 71 && count == 4 && words[at + 2] == 1 {
                spec_ids.push(words[at + 3]);
            }
            at += count.max(1);
        }
        for (id, _) in VULKAN_SPEC_CONSTANTS {
            assert!(
                spec_ids.contains(&id),
                "the fragment SPIR-V has no SpecId {id} — it was not regenerated from the \
                 GLSL (scripts/regen-spirv.sh). Found: {spec_ids:?}"
            );
        }
    }

    /// The shared buffer is exactly the regions it is asked to hold, and each region
    /// starts where the one before it ends.
    ///
    /// Written as a running total rather than as a sum, so adding a region means
    /// extending the chain rather than rewriting an equation — plan-116-F added the
    /// third and found this asserting the two-region total. bug-688 added the band index
    /// and the picture table and reordered the regions to Metal's order; the bases the
    /// shader reads are specialized from these constants, so the chain is the whole
    /// layout contract.
    #[test]
    fn the_shared_buffer_holds_every_region() {
        let mut end = VULKAN_EDGE_BYTES;
        // bug-687 put the edge residuals between the edges and the band index.
        assert_eq!(
            VULKAN_EDGE_RESIDUAL_BASE_WORDS * 4,
            end,
            "the edge-residual region must start where the edge region ends",
        );
        end += VULKAN_EDGE_RESIDUAL_BYTES;
        assert_eq!(
            VULKAN_BAND_BASE_WORDS * 4,
            end,
            "the band region must start where the edge-residual region ends",
        );
        end += VULKAN_MAX_FRAME_BAND_WORDS * 4;
        assert_eq!(
            VULKAN_GRADIENT_BASE_WORDS * 4,
            end,
            "the gradient region must start where the band region ends",
        );
        end += VULKAN_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4;
        // plan-163-B put the dash words between the gradient stops and the glyphs.
        assert_eq!(
            VULKAN_DASH_BASE_WORDS * 4,
            end,
            "the dash region must start where the gradient region ends",
        );
        end += VULKAN_MAX_FRAME_DASH_WORDS * 4;
        assert_eq!(
            VULKAN_GLYPH_BASE_WORDS * 4,
            end,
            "the glyph region must start where the dash region ends",
        );
        end += VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4;
        assert_eq!(
            VULKAN_PICTURE_RECORDS_OFFSET, end,
            "the picture records must start where the glyph region ends",
        );
        end += VULKAN_PICTURE_RECORDS * PICTURE_RECORD_BYTES;
        assert_eq!(
            VULKAN_PICTURE_INDEX_OFFSET, end,
            "the picture index must start where the records end",
        );
        end += VULKAN_PICTURE_INDEX_SLOTS * 4;
        assert_eq!(
            VULKAN_BUFFER_BYTES, end,
            "the buffer must be exactly its regions, with nothing past the last",
        );
        // Every region base the shader indexes is an `int`, and so is every index it
        // computes inside the glyph region.
        assert!(
            VULKAN_PICTURE_RECORDS_OFFSET / 4 < i32::MAX as usize,
            "a word index past i32::MAX cannot be read by the GLSL's `int` arithmetic"
        );
    }
}
