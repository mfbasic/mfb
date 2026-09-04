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
use crate::codegen::runtime::canvas::{
    push_symbol_address, BLEND_MODE_COUNT, CANVAS_ITEM_BUFFER_BYTES, CANVAS_MAX_FRAME_ITEMS,
    EDGE_SLOTS, FIXED_POINT_SCALE, GEO_KIND_POLYGON, GEO_KIND_TEXT, GLYPH_META_H, GLYPH_META_SLOTS,
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
    GRAPHICS_OFFSET_VULKAN_QUEUE, GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY,
    GRAPHICS_OFFSET_VULKAN_READY, GRAPHICS_OFFSET_VULKAN_READ_BUFFER,
    GRAPHICS_OFFSET_VULKAN_READ_MEMORY, GRAPHICS_OFFSET_VULKAN_RENDER_PASS,
    GRAPHICS_OFFSET_VULKAN_SET_LAYOUT, GRAPHICS_OFFSET_VULKAN_TEX_HEIGHT,
    GRAPHICS_OFFSET_VULKAN_TEX_WIDTH, GRAPHICS_STATE_SYMBOL, HEADER_AUX0, HEADER_AUX1,
    HEADER_BLEND, HEADER_BOUNDS, HEADER_CAP, HEADER_CAP_END_X, HEADER_CAP_START_X, HEADER_CLIP_X0,
    HEADER_CLIP_X1, HEADER_CLIP_Y0, HEADER_CLIP_Y1, HEADER_ELLIPSE_COS, HEADER_ELLIPSE_SIN,
    HEADER_FILL_R, HEADER_GRADIENT_COUNT, HEADER_GRADIENT_FROM_X, HEADER_GRADIENT_KIND,
    HEADER_HAS_TRANSFORM, HEADER_KIND, HEADER_RADIUS, HEADER_SHAPE, HEADER_SLOTS,
    HEADER_STROKE_HALF, HEADER_STROKE_R, HEADER_TRANSFORM_IA, HEADER_TRANSFORM_IB,
    HEADER_TRANSFORM_IC, HEADER_TRANSFORM_ID, HEADER_TRANSFORM_ITX, HEADER_TRANSFORM_ITY,
    ITEM_ARC_CAP, ITEM_ARC_EDGE_BASE, ITEM_ARC_GLYPH_HEIGHT, ITEM_BLOCK_SIZE,
    ITEM_ELLIPSE_GRADIENT_BASE, ITEM_ELLIPSE_GRADIENT_COUNT, ITEM_OFFSET_ARC, ITEM_OFFSET_ARC_CAPS,
    ITEM_OFFSET_CLIP, ITEM_OFFSET_ELLIPSE, ITEM_OFFSET_FILL, ITEM_OFFSET_GRADIENT,
    ITEM_OFFSET_MISC, ITEM_OFFSET_QUAD, ITEM_OFFSET_SHAPE, ITEM_OFFSET_STROKE, ITEM_OFFSET_SURFACE,
    ITEM_OFFSET_TRANSFORM, ITEM_SURFACE_BLEND, ITEM_SURFACE_GRADIENT_KIND,
    MAX_FRAME_GRADIENT_STOPS, VULKAN_BUFFER_BYTES, VULKAN_GLYPH_BASE_WORDS,
    VULKAN_GRADIENT_BASE_WORDS, VULKAN_MAX_FRAME_EDGES, VULKAN_MAX_FRAME_GLYPH_SAMPLES,
};
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;

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
// almost always breaks: members inside their struct, no two overlapping, sizes a
// multiple of 8.
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
// `pPushConstantRanges` (offset 40) is deliberately absent: `emit_struct` zeroes the
// whole struct, and with `rangeCount` 0 the pointer must be null. plan-116-A moved the
// item block out of the push constants, so there is no range to point at.

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

/// `canvas::vulkanReady() AS Boolean` — build the device and queue, once.
///
/// The Vulkan twin of `canvas::metalReady`, and the second half of the renderer
/// branch's condition. Tri-state memo in `GRAPHICS_OFFSET_VULKAN_READY`: a machine
/// with a loader but no ICD pays the enumeration once, not per frame.
///
/// Picks the **first** physical device and its first graphics-capable queue family.
/// A machine with a discrete and an integrated GPU gets whichever the loader lists
/// first, which is the loader's own preference order — and for a renderer whose
/// output is checked against a software oracle within a tolerance, "which GPU" is not
/// a question this letter needs an opinion on.
pub(crate) fn emit_vulkan_ready(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    if !has_vulkan_backend(platform) {
        builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return Ok(());
    }
    let unavailable = builder.label("vk_ready_unavailable");
    let done = builder.label("vk_ready_done");
    let build = builder.label("vk_ready_build");
    let already_built = builder.label("vk_ready_already_built");
    let family_head = builder.label("vk_family_head");
    let family_found = builder.label("vk_family_found");

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

    // Already tried? Report what happened last time.
    //
    // **Both compares happen before the answer is written, and the state rides a fresh
    // vreg.** `RESULT_VALUE_REGISTER` is `mfb_return(1)`, which on x86-64 SysV realizes
    // to `rsi` — and so does `SCRATCH[1]` (`map_scratch_register(10)`). Writing the
    // answer between the two compares therefore *overwrote the value being compared*,
    // and the second compare tested the answer against itself: every call after the
    // first returned TRUE, whatever the tri-state actually said. On AArch64 the two are
    // `x1` and `x10`, so the sequence is correct there and the fault is invisible on
    // the development host.
    //
    // Measured on box 2227, which has no Vulkan driver at all: the build failed and
    // stored 2, the renderer correctly fell back to software — and then
    // `__canvas_writeStats` asked the same question and was told TRUE.
    let ready = builder.temporary_vreg();
    state_base_into(builder, off_state);
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_state,
    ));
    builder.emit(abi::load_u64(
        &ready,
        abi::SCRATCH[0],
        GRAPHICS_OFFSET_VULKAN_READY,
    ));
    builder.emit(abi::compare_immediate(&ready, "0"));
    builder.emit(abi::branch_eq(&build));
    builder.emit(abi::compare_immediate(&ready, "1"));
    builder.emit(abi::branch_eq(&already_built));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&already_built));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
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

    // Remember the library handle, mark ready, answer yes.
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
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "1"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READY,
        abi::SCRATCH[0],
    );
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&unavailable));
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "2"));
    emit_state_store(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READY,
        abi::SCRATCH[0],
    );
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));

    builder.emit(abi::label(&done));
    Ok(())
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
        MEMORY_HOST_VISIBLE_COHERENT,
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
        &CANVAS_ITEM_BUFFER_BYTES.to_string(),
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
        MEMORY_HOST_VISIBLE_COHERENT,
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
        &CANVAS_ITEM_BUFFER_BYTES.to_string(),
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
            CANVAS_ITEM_BUFFER_BYTES,
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
            (LAYOUT_INFO_RANGE_COUNT, Field::U32("0")),
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
    debug_assert_eq!(
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
        MEMORY_DEVICE_LOCAL,
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
        MEMORY_HOST_VISIBLE_COHERENT,
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
    required: &str,
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
    required: &str,
    unavailable: &str,
) -> Result<(), String> {
    let head = builder.label("vk_memtype_head");
    let next = builder.label("vk_memtype_next");
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
    builder.emit(abi::branch_ge(unavailable));

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
    builder.emit(abi::branch_eq(&found));

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

    builder.emit(abi::label(&found));
    Ok(())
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

    // The 16.16 fields: bounds, shape parameters, arc angles.
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
        (
            ITEM_OFFSET_SHAPE,
            [
                HEADER_SHAPE,
                HEADER_SHAPE + 1,
                HEADER_SHAPE + 2,
                HEADER_SHAPE + 3,
            ],
        ),
        (
            ITEM_OFFSET_ARC,
            [HEADER_AUX0, HEADER_AUX1, HEADER_AUX1, HEADER_AUX1],
        ),
        // plan-116-D: an arc's two sweep endpoints, four consecutive header slots
        // narrowing to 16.16 exactly like the bounds — so the new `ivec4` rides this
        // loop rather than adding a hand-written store per coordinate. Written for
        // every kind; only a Round-capped arc reads them.
        (
            ITEM_OFFSET_ARC_CAPS,
            [
                HEADER_CAP_START_X,
                HEADER_CAP_START_X + 1,
                HEADER_CAP_END_X,
                HEADER_CAP_END_X + 1,
            ],
        ),
        // plan-116-E: an ellipse's rotation as cos, sin. The trailing pair repeats the
        // sine rather than naming an unused slot, because this loop writes four words
        // and the shader reads only x and y — the same shape the arc row above uses.
        (
            ITEM_OFFSET_ELLIPSE,
            [
                HEADER_ELLIPSE_COS,
                HEADER_ELLIPSE_SIN,
                HEADER_ELLIPSE_SIN,
                HEADER_ELLIPSE_SIN,
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
        // plan-116-B: the clip is already RESOLVED to x0,y0,x1,y1 in the header, so it
        // rides this loop unchanged — four consecutive slots narrowing to 16.16 like
        // the bounds above, and no arithmetic repeated per item.
        (
            ITEM_OFFSET_CLIP,
            [
                HEADER_CLIP_X0,
                HEADER_CLIP_Y0,
                HEADER_CLIP_X1,
                HEADER_CLIP_Y1,
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

    // misc = { kind, radius (16.16), strokeHalf (16.16), edgeCount }
    for (index, slot, fixed) in [
        (0usize, HEADER_KIND, false),
        (1, HEADER_RADIUS, true),
        (2, HEADER_STROKE_HALF, true),
        (3, HEADER_AUX0, false),
    ] {
        builder.emit(abi::load_double(abi::FP_SCRATCH[1], header, slot * 8));
        if fixed {
            builder.emit(abi::float_multiply_d(
                abi::FP_SCRATCH[1],
                abi::FP_SCRATCH[1],
                scale,
            ));
            builder.emit(abi::float_round_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
        } else {
            builder.emit(abi::float_convert_to_signed_x(
                abi::SCRATCH[1],
                abi::FP_SCRATCH[1],
            ));
        }
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
    builder.emit(abi::load_double(
        abi::FP_SCRATCH[1],
        header,
        HEADER_GRADIENT_KIND * 8,
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
        &MAX_FRAME_GRADIENT_STOPS.to_string(),
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
) {
    let head = builder.label("vk_edge_head");
    let done = builder.label("vk_edge_done");
    let empty = builder.label("vk_edge_empty");
    let copy = builder.label("vk_edge_copy");

    let count = abi::SCRATCH[2];
    let index = abi::SCRATCH[3];
    let source = abi::SCRATCH[4];
    let target = abi::SCRATCH[5];
    let scale = abi::FP_SCRATCH[0];

    // Only a polygon has edges. Every other kind leaves `misc.w` holding whatever
    // `HEADER_AUX0` meant for it — an arc's start angle — which is harmless because
    // the shader reads `misc.w` only in the polygon arm, but zeroing it keeps the
    // block meaning one thing at a time.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], GEO_KIND_POLYGON));
    builder.emit(abi::branch_ne(&empty));

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
    // Unreachable: `__canvas_vulkanRenderable` already declined any scene whose
    // polygon edges do not fit, so the whole frame went to the software rasteriser.
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
    // into the item block above.
    builder.emit(abi::add_registers(abi::SCRATCH[0], abi::SCRATCH[0], count));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_edge_cursor,
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

    builder.emit(abi::label(&head));
    builder.emit(abi::compare_registers(index, count));
    builder.emit(abi::branch_ge(&done));
    // out[0..1] = (x0, y0); out[2..3] = (x0 + dx, y0 + dy)
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
    }
    builder.emit(abi::add_immediate(source, source, EDGE_SLOTS * 8));
    builder.emit(abi::add_immediate(index, index, 1));
    builder.emit(abi::branch(&head));

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
/// same number `__canvas_vulkanRenderable` sums against `CANVAS_MAX_FRAME_ITEMS`, so
/// the two cannot disagree about what "full" means.
///
/// Branches to `full` without writing or advancing if the frame is already at
/// capacity. Unreachable in the same way `emit_edge_upload`'s bound is: the predicate
/// declined any frame with too many quads, so the whole scene went to software. Kept
/// because the alternative to declining is a write past the mapping.
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
        &CANVAS_MAX_FRAME_ITEMS.to_string(),
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
    debug_assert_eq!(
        ITEM_BLOCK_SIZE % 8,
        0,
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

    // Split only when the mode is not Normal...
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item_mode,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&single));
    // ...and the item actually strokes (`strokeHalf` > 0, in 16.16)...
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_MISC + 8,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_le(&single));
    // ...and actually fills (fill alpha > 0). A `Line` or an `Arc` reaches here with
    // its stroke colour already moved into the fill slots and `strokeHalf` negative
    // (`__canvas_strokeAsFill`), so it is fill-only and takes the single path.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_item + ITEM_OFFSET_FILL + 12,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_le(&single));

    // Record one: the fill, with the stroke switched off.
    //
    // `strokeHalf` is parked on the STACK, not in a scratch register.
    // `emit_item_publish` uses `SCRATCH[0..3]` — `SCRATCH[1]` is its target pointer —
    // so a register saved across it comes back holding a mapped address. Measured
    // rather than reasoned about: doing exactly that gave record two a `strokeHalf` of
    // whatever the buffer pointer was, i.e. an enormous stroke band, and the Vulkan
    // harness reported `worst=215` with the GPU painting stroke pixels the oracle
    // never touched.
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
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
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
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
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
/// `vkCmdDraw(cmd, 4, count, 0, firstInstance)` — four synthesised corners, `count`
/// instances, and the run's base as `firstInstance`. Vulkan's `gl_InstanceIndex`
/// *includes* `firstInstance`, so each instance reads exactly the block it was
/// published into with no index arithmetic in the shader.
///
/// A run ends at a glyph run or at the end of the scene, and nowhere else. Nothing
/// per-item is bound between instances any more — the edges were already a buffer
/// region and the item block just became one — so consecutive shapes have nothing left
/// to separate them.
///
/// Emits nothing if the run is empty, which is the case at the very start of a frame
/// and after two glyph runs in a row; `vkCmdDraw` with `instanceCount = 0` is legal but
/// this keeps the command stream honest.
fn emit_run_flush(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    off_cmd_handle: usize,
    off_draw_fn: usize,
    off_item_cursor: usize,
    off_run_start: usize,
    off_run_count: usize,
) {
    let empty = builder.label("vk_run_empty");

    // The count is computed into a **stack slot**, and every one of the five arguments
    // below is then staged from memory. That is not ceremony: on x86-64 the scratch
    // pool aliases the C argument bank (`map_scratch_register` — `SCRATCH[3]` is `r8`,
    // which is `c_arg(4)`), so a count or a base held in a scratch register across the
    // staging is one an earlier argument's `move` destroys. Sourcing every argument
    // from the stack makes the staging order irrelevant, which is the only version of
    // this that stays correct when a later letter adds a sixth argument.
    let cursor = builder.temporary_vreg();
    let start = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &cursor,
        abi::stack_pointer(),
        off_item_cursor,
    ));
    builder.emit(abi::load_u64(&start, abi::stack_pointer(), off_run_start));
    builder.emit(abi::compare_registers(&cursor, &start));
    builder.emit(abi::branch_le(&empty));
    builder.emit(abi::subtract_registers(&cursor, &cursor, &start));
    builder.emit(abi::store_u64(&cursor, abi::stack_pointer(), off_run_count));

    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "4")); // vertexCount
    builder.emit(abi::load_u64(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_run_count,
    )); // instanceCount
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0")); // firstVertex
    emit_int_arg_slot(builder, platform, 4, off_run_start); // firstInstance
    emit_call_fn(builder, off_draw_fn);

    builder.emit(abi::label(&empty));
    // The next run starts wherever this frame has published to, whether or not
    // anything was drawn just now.
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
}

/// `canvas::vulkanDrawScene(surface, width, height, geometry, offsets)` — render one
/// frame on the GPU and read it back into `surface`.
///
/// The Vulkan counterpart of `canvas::metalDrawScene`, and deliberately the same
/// shape: draw offscreen, read back, and let the frame leave through the same
/// `canvas::blitSurface` every other path uses. That is what makes the two backends
/// and the software oracle comparable at all — the tolerance comparator diffs an
/// RGBA8 buffer — and here it is also what makes the renderer testable, because no
/// reachable Linux box has a display server for a swapchain to present to.
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
struct GlyphDrawSlots {
    state: usize,
    item: usize,
    header: usize,
    width: usize,
    height: usize,
    cmd_handle: usize,
    draw_fn: usize,
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
fn emit_glyph_draws(builder: &mut CodeBuilder, platform: &dyn CodegenPlatform, at: GlyphDrawSlots) {
    let head = builder.label("vk_glyph_head");
    let done = builder.label("vk_glyph_done");
    let next = builder.label("vk_glyph_next");
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
    // built. It draws nothing rather than reading the metadata list out of range.
    builder.emit(abi::compare_immediate(abi::SCRATCH[2], "0"));
    builder.emit(abi::branch_lt(&next));

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
    // nothing to draw.
    builder.emit(abi::load_u64(
        abi::SCRATCH[5],
        abi::stack_pointer(),
        at.glyph_w,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[5], "0"));
    builder.emit(abi::branch_le(&next));
    builder.emit(abi::load_u64(
        abi::SCRATCH[6],
        abi::stack_pointer(),
        at.glyph_h,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[6], "0"));
    builder.emit(abi::branch_le(&next));

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
    builder.emit(abi::branch_gt(&next));

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

    // One instance, not a run: a glyph run is N draws by design (`GEO_KIND_TEXT`), and
    // folding it into the instancing scheme is a change of shape rather than of
    // transport. The block still rides the buffer, so nothing here is per-draw state
    // any more except the index itself.
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        at.cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "4"));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "1"));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    emit_int_arg_slot(builder, platform, 4, at.instance);
    emit_call_fn(builder, at.draw_fn);

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
    surface: &Operand,
    width: &Operand,
    height: &Operand,
    geometry: &Operand,
    offsets: &Operand,
    glyph_meta: &Operand,
    glyph_coverage: &Operand,
) -> Result<(), String> {
    if !has_vulkan_backend(platform) {
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

    // Park the arguments before anything calls.
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

    // The renderer must be ready. `canvas::vulkanReady` is the branch's own
    // condition, so this is a guard against being called out of order, not a
    // fallback: a frame that got here without a pipeline renders nothing and leaves
    // the cleared surface the software path would have produced.
    state_base_into(builder, off_state);
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_READY,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "1"));
    builder.emit(abi::branch_ne(&done));
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
    for (field, slot) in [
        (PASS_BEGIN_RENDER_PASS, GRAPHICS_OFFSET_VULKAN_RENDER_PASS),
        (PASS_BEGIN_FRAMEBUFFER, GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER),
    ] {
        emit_state_load(builder, off_state, slot, abi::SCRATCH[0]);
        builder.emit(abi::store_u64(
            abi::SCRATCH[0],
            abi::stack_pointer(),
            off_pass_begin + field,
        ));
    }
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

        emit_run_flush(
            builder,
            platform,
            off_cmd_handle,
            off_draw_fn,
            off_item_cursor,
            off_run_start,
            off_run_count,
        );
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
    emit_edge_upload(builder, off_state, off_item, off_header, off_edge_cursor);
    emit_gradient_upload(builder, off_state, off_item, off_header, off_grad_cursor);
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

    // A glyph run ends the instanced run: its quads are N draws rather than N
    // instances (they are still one block each, at the same cursor), so the shapes
    // accumulated so far have to reach the command stream before them or they would
    // be drawn out of order — on top of the text instead of under it.
    builder.emit(abi::label(&text_item));
    emit_run_flush(
        builder,
        platform,
        off_cmd_handle,
        off_draw_fn,
        off_item_cursor,
        off_run_start,
        off_run_count,
    );
    emit_glyph_draws(
        builder,
        platform,
        GlyphDrawSlots {
            state: off_state,
            item: off_item,
            header: off_header,
            width: off_width,
            height: off_height,
            cmd_handle: off_cmd_handle,
            draw_fn: off_draw_fn,
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

    // The scene's last run — everything published since the final glyph run, or the
    // whole frame when it contains no text. Without this the trailing shapes are
    // written into the buffer and never drawn.
    emit_run_flush(
        builder,
        platform,
        off_cmd_handle,
        off_draw_fn,
        off_item_cursor,
        off_run_start,
        off_run_count,
    );

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

    // --- copy the image out -------------------------------------------------------
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
    value: impl Into<Operand>,
) {
    let value = value.into();
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
    dst: impl Into<Operand>,
) {
    // A fresh vreg, not a fixed `SCRATCH[k]` — the same rule `emit_call_fn` follows,
    // and for a sharper reason here. On x86-64 the scratch pool aliases the C argument
    // bank (`map_scratch_register`): `SCRATCH[3]` is `r8`, which is `c_arg(4)`. This
    // helper is called *between* argument stagings, so a fixed scratch silently
    // overwrote an argument already in place — `vkCmdBindDescriptorSets` received the
    // graphics-state pointer as its `descriptorSetCount` and walked a one-element
    // array for a dozen entries. On AArch64 the two banks are disjoint, so the fault
    // does not exist on the development host.
    let base = builder.temporary_vreg();
    builder.emit(abi::load_u64(&base, abi::stack_pointer(), state_slot));
    builder.emit(abi::load_u64(dst, &base, offset));
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
    value: impl Into<Operand>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::runtime::canvas::VULKAN_EDGE_BYTES;
    use crate::codegen::runtime::canvas::{
        GRAPHICS_OFFSET_MTL_PIPELINE_MODES, GRAPHICS_STATE_SIZE,
    };

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
            CANVAS_ITEM_BUFFER_BYTES,
            CANVAS_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE,
            "the buffer must hold exactly the number of records the predicates admit"
        );
    }

    /// The GLSL's `GRADIENT_BASE` is `VULKAN_GRADIENT_BASE_WORDS` (plan-116-F).
    ///
    /// The same arrangement `GLYPH_BASE` has and for the same reason: the shader cannot
    /// see a Rust constant and the SPIR-V is checked in, so this is the only thing
    /// standing between the two numbers. A disagreement would not fail anywhere —
    /// every gradient would simply read its stops from the wrong place in a buffer that
    /// is entirely valid memory, and render a plausible wrong ramp.
    #[test]
    fn the_shaders_gradient_base_matches_the_buffer_layout() {
        const GLSL: &str = include_str!("shaders/mfb_canvas.frag");
        let want = format!("const int GRADIENT_BASE = {VULKAN_GRADIENT_BASE_WORDS};");
        assert!(
            GLSL.contains(&want),
            "the GLSL declares a gradient-region base that is not \
             VULKAN_GRADIENT_BASE_WORDS ({VULKAN_GRADIENT_BASE_WORDS})"
        );
    }

    /// The GLSL's `GLYPH_BASE` is `VULKAN_GLYPH_BASE_WORDS`.
    ///
    /// The shader cannot see a Rust constant and the SPIR-V is checked in, so this is
    /// the only thing standing between the two numbers. A disagreement would not fail
    /// anywhere: every glyph would simply read coverage from the wrong place in a buffer
    /// that is entirely valid memory, and the frame would come back full of noise —
    /// which looks like a rasteriser bug, on a machine with a Vulkan driver.
    #[test]
    fn the_shaders_glyph_base_matches_the_buffer_layout() {
        const GLSL: &str = include_str!("shaders/mfb_canvas.frag");
        let line = GLSL
            .lines()
            .find(|l| l.trim_start().starts_with("const int GLYPH_BASE"))
            .expect("the fragment shader declares GLYPH_BASE");
        let declared: usize = line
            .split('=')
            .nth(1)
            .and_then(|rhs| rhs.trim().trim_end_matches(';').parse().ok())
            .unwrap_or_else(|| panic!("cannot read a number from `{line}`"));
        assert_eq!(
            declared, VULKAN_GLYPH_BASE_WORDS,
            "the shader reads glyph coverage from word {declared}, the emitter writes it \
             at word {VULKAN_GLYPH_BASE_WORDS}",
        );
    }

    /// The shared buffer is exactly the regions it is asked to hold, and each region
    /// starts where the one before it ends.
    ///
    /// Written as a running total rather than as a sum, so adding a fourth region means
    /// extending the chain rather than rewriting an equation — plan-116-F added the
    /// third and found this asserting the two-region total.
    #[test]
    fn the_shared_buffer_holds_every_region() {
        assert_eq!(
            VULKAN_GLYPH_BASE_WORDS * 4,
            VULKAN_EDGE_BYTES,
            "the glyph region must start where the edge region ends",
        );
        assert_eq!(
            VULKAN_GRADIENT_BASE_WORDS * 4,
            VULKAN_EDGE_BYTES + VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4,
            "the gradient region must start where the glyph region ends",
        );
        assert_eq!(
            VULKAN_BUFFER_BYTES,
            VULKAN_GRADIENT_BASE_WORDS * 4 + MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4,
            "the buffer must be exactly its three regions, with nothing past the last",
        );
    }
}
