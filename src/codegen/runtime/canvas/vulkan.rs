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
    push_symbol_address, EDGE_SLOTS, FIXED_POINT_SCALE, GEO_KIND_POLYGON,
    GRAPHICS_OFFSET_VULKAN_COMMAND_BUFFER, GRAPHICS_OFFSET_VULKAN_COMMAND_POOL,
    GRAPHICS_OFFSET_VULKAN_DESC_POOL, GRAPHICS_OFFSET_VULKAN_DESC_SET,
    GRAPHICS_OFFSET_VULKAN_DEVICE, GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER,
    GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED, GRAPHICS_OFFSET_VULKAN_EDGE_MEMORY,
    GRAPHICS_OFFSET_VULKAN_FRAMEBUFFER, GRAPHICS_OFFSET_VULKAN_IMAGE,
    GRAPHICS_OFFSET_VULKAN_IMAGE_MEMORY, GRAPHICS_OFFSET_VULKAN_IMAGE_VIEW,
    GRAPHICS_OFFSET_VULKAN_INSTANCE, GRAPHICS_OFFSET_VULKAN_LIB, GRAPHICS_OFFSET_VULKAN_MAPPED,
    GRAPHICS_OFFSET_VULKAN_PHYSICAL, GRAPHICS_OFFSET_VULKAN_PIPELINE,
    GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT, GRAPHICS_OFFSET_VULKAN_QUEUE,
    GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY, GRAPHICS_OFFSET_VULKAN_READY,
    GRAPHICS_OFFSET_VULKAN_READ_BUFFER, GRAPHICS_OFFSET_VULKAN_READ_MEMORY,
    GRAPHICS_OFFSET_VULKAN_RENDER_PASS, GRAPHICS_OFFSET_VULKAN_SET_LAYOUT,
    GRAPHICS_OFFSET_VULKAN_TEX_HEIGHT, GRAPHICS_OFFSET_VULKAN_TEX_WIDTH, GRAPHICS_STATE_SYMBOL,
    HEADER_AUX0, HEADER_AUX1, HEADER_BOUNDS, HEADER_FILL_R, HEADER_KIND, HEADER_RADIUS,
    HEADER_SHAPE, HEADER_SLOTS, HEADER_STROKE_HALF, HEADER_STROKE_R, ITEM_ARC_EDGE_BASE,
    ITEM_BLOCK_SIZE, ITEM_OFFSET_ARC, ITEM_OFFSET_FILL, ITEM_OFFSET_MISC, ITEM_OFFSET_QUAD,
    ITEM_OFFSET_SHAPE, ITEM_OFFSET_STROKE, ITEM_OFFSET_SURFACE, VULKAN_EDGE_BYTES,
    VULKAN_MAX_FRAME_EDGES,
};
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;

/// The Vulkan loader's soname on Linux.
pub(crate) const VULKAN_SONAME: &str = "libvulkan.so.1";
/// `RTLD_NOW`; `RTLD_LOCAL` is 0.
const RTLD_NOW: &str = "2";

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
/// exactly where the software oracle's `__CANVAS_SRGB` table does, and the same
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
pub(crate) fn data_objects() -> Vec<CodeDataObject> {
    let mut objects = vec![CodeDataObject {
        symbol: soname_symbol(),
        kind: "raw".to_string(),
        layout: "C string (NUL-terminated)".to_string(),
        align: 1,
        size: VULKAN_SONAME.len() + 1,
        value: hex_encode_cstring(VULKAN_SONAME),
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

/// `VkPushConstantRange`, 12 bytes.
const PUSH_RANGE_SIZE: usize = 12;
const PUSH_RANGE_STAGES: usize = 0;
const PUSH_RANGE_OFFSET: usize = 4;
const PUSH_RANGE_BYTES: usize = 8;

/// `VkPipelineLayoutCreateInfo`, 48 bytes.
const LAYOUT_INFO_SIZE: usize = 48;
const LAYOUT_INFO_STYPE: usize = 0;
const LAYOUT_INFO_SET_COUNT: usize = 20;
const LAYOUT_INFO_SETS: usize = 24;
const LAYOUT_INFO_RANGE_COUNT: usize = 32;
const LAYOUT_INFO_RANGES: usize = 40;

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
    if !matches!(platform.family(), PlatformFamily::Linux) {
        builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return Ok(());
    }
    let symbol = builder.current_symbol.clone();
    let unavailable = builder.label("vk_ready_unavailable");
    let done = builder.label("vk_ready_done");
    let build = builder.label("vk_ready_build");
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
    state_base_into(builder, off_state);
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_state,
    ));
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::SCRATCH[0],
        GRAPHICS_OFFSET_VULKAN_READY,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], "0"));
    builder.emit(abi::branch_eq(&build));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    builder.emit(abi::compare_immediate(abi::SCRATCH[1], "1"));
    builder.emit(abi::branch_eq(&done));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&build));

    // handle = dlopen("libvulkan.so.1", RTLD_NOW)
    emit_data_address(
        &symbol,
        abi::c_arg(0),
        &soname_symbol(),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", RTLD_NOW));
    platform.emit_external_call(
        "dlopen",
        &symbol,
        platform_imports,
        &mut builder.instructions,
        &mut builder.relocations,
    )?;
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
/// One binding, one set, one descriptor — a `readonly buffer` the fragment shader
/// walks when `kind` is `Polygon`. It exists only because a polygon carries an
/// unbounded number of edges: everything else about an item fits the 112-byte push
/// constant block, and the guaranteed push-constant range is 128.
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
    let off_binding = builder.allocate_stack_object("vk_binding", BINDING_SIZE);
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
    let off_desc_buffer = builder.allocate_stack_object("vk_desc_buffer", DESC_BUFFER_INFO_SIZE);
    let off_write_set = builder.allocate_stack_object("vk_write_set", WRITE_SET_SIZE);

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
        off_set_layout_info,
        SET_LAYOUT_INFO_SIZE,
        &[
            (0, Field::U32(ST_DESCRIPTOR_SET_LAYOUT_CREATE_INFO)),
            (SET_LAYOUT_BINDING_COUNT, Field::U32("1")),
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
            (POOL_SIZE_COUNT, Field::U32("1")),
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

    // --- the edge buffer itself ----------------------------------------------------
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
        &VULKAN_EDGE_BYTES.to_string(),
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
        &VULKAN_EDGE_BYTES.to_string(),
    ));
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "0")); // flags
    builder.emit(abi::add_immediate(
        abi::c_arg(5),
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
        GRAPHICS_OFFSET_VULKAN_EDGE_MAPPED,
        abi::SCRATCH[0],
    );

    // --- point the set at the buffer, once -----------------------------------------
    // Zero it first. `offset` sits between `buffer` and `range` and is not written
    // below, and a storage buffer's offset must be a multiple of
    // `minStorageBufferOffsetAlignment` — so leftover stack garbage there is not a
    // wrong picture, it is a rejected descriptor write and a blank frame.
    emit_struct(builder, off_desc_buffer, DESC_BUFFER_INFO_SIZE, &[]);
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_EDGE_BUFFER,
        abi::SCRATCH[0],
    );
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_desc_buffer + DESC_BUFFER_INFO_BUFFER,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[0],
        "Integer",
        &VULKAN_EDGE_BYTES.to_string(),
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_desc_buffer + DESC_BUFFER_INFO_RANGE,
    ));
    emit_struct(
        builder,
        off_write_set,
        WRITE_SET_SIZE,
        &[
            (0, Field::U32(ST_WRITE_DESCRIPTOR_SET)),
            (WRITE_SET_BINDING, Field::U32("0")),
            (WRITE_SET_COUNT, Field::U32("1")),
            (WRITE_SET_TYPE, Field::U32(DESCRIPTOR_TYPE_STORAGE_BUFFER)),
            (WRITE_SET_BUFFER_INFO, Field::Addr(off_desc_buffer)),
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
        off_write_set + WRITE_SET_DST,
    ));
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
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "1"));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_write_set,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
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
    let off_push_range = builder.allocate_stack_object("vk_push_range", PUSH_RANGE_SIZE);
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

    // --- pipeline layout: one push-constant range, both stages --------------------
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
            (PUSH_RANGE_BYTES, Field::U32(&ITEM_BLOCK_SIZE.to_string())),
        ],
    );
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
    emit_struct(
        builder,
        off_blend_attachment,
        BLEND_ATTACHMENT_SIZE,
        &[
            (BLEND_ENABLE, Field::U32("1")),
            (BLEND_SRC_COLOR, Field::U32(BLEND_FACTOR_ONE)),
            (
                BLEND_DST_COLOR,
                Field::U32(BLEND_FACTOR_ONE_MINUS_SRC_ALPHA),
            ),
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
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(5),
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
        GRAPHICS_OFFSET_VULKAN_PIPELINE,
        abi::SCRATCH[0],
    );
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
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "0")); // flags
    builder.emit(abi::add_immediate(
        abi::c_arg(5),
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

/// Fill the 112-byte item block at `sp + off_item` from the geometry header whose
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
}

/// Append this item's polygon edges to the frame's edge buffer and record where they
/// landed.
///
/// **Why an offset rather than a rebind.** A Vulkan command buffer is recorded now
/// and executed later, all at once. There is one edge buffer for the frame, so
/// rewriting it per item — or rebinding the same buffer per item — would give every
/// polygon whatever the *last* one wrote. Each polygon therefore takes a slice and
/// carries its start index in the item block, which is a per-draw push constant and
/// so really is per-item. Metal has the opposite property and needs none of this:
/// `setFragmentBytes:` copies the bytes into the command buffer at record time.
///
/// The geometry cache stores each edge as `x0, y0, dx, dy, invLenSq` doubles; the
/// shader wants the two endpoints in 16.16, and recomputes the rest. `invLenSq` is
/// the one quantity fixed point represents badly — a 100-px edge gives 1e-4, which
/// is 6 in 16.16 — and a GPU has the divide for free, so dropping it from the
/// payload is both smaller and more accurate.
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
pub(crate) fn emit_vulkan_draw_scene(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    surface: &Operand,
    width: &Operand,
    height: &Operand,
    geometry: &Operand,
    offsets: &Operand,
) -> Result<(), String> {
    if !matches!(platform.family(), PlatformFamily::Linux) {
        return Ok(());
    }
    let done = builder.label("vk_draw_done");
    let unavailable = builder.label("vk_draw_unavailable");
    let item_head = builder.label("vk_draw_item_head");
    let item_done = builder.label("vk_draw_item_done");
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
    let off_header = builder.allocate_stack_object("vk_header", 8);

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
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "1")); // setCount
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
    builder.emit(abi::add_immediate(
        abi::c_arg(5),
        abi::stack_pointer(),
        off_desc_set_handle,
    ));
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
    emit_item_block(builder, off_item, off_width, off_height);
    emit_edge_upload(builder, off_state, off_item, off_header, off_edge_cursor);

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdPushConstants",
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
        GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT,
        abi::c_arg(1),
    );
    builder.emit(abi::move_immediate(
        abi::c_arg(2),
        "Integer",
        SHADER_STAGE_VERTEX_AND_FRAGMENT,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    builder.emit(abi::move_immediate(
        abi::c_arg(4),
        "Integer",
        &ITEM_BLOCK_SIZE.to_string(),
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(5),
        abi::stack_pointer(),
        off_item,
    ));
    emit_call_fn(builder, off_fn);

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCmdDraw",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_cmd_handle,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "4")); // vertices
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "1")); // instances
    builder.emit(abi::move_immediate(abi::c_arg(3), "Integer", "0"));
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "0"));
    emit_call_fn(builder, off_fn);

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
    builder.emit(abi::move_immediate(abi::c_arg(4), "Integer", "1"));
    builder.emit(abi::add_immediate(
        abi::c_arg(5),
        abi::stack_pointer(),
        off_copy,
    ));
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
    let register_args = platform
        .backend()
        .register_model()
        .external_int_argument_registers();
    if n < register_args {
        builder.emit(abi::move_immediate(abi::c_arg(n), "Integer", "0"));
        return;
    }
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
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
    platform.emit_external_call(
        "dlsym",
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

    /// The push-constant block is the size both shaders declare, and fits Vulkan's
    /// guaranteed range.
    ///
    /// 128 bytes is the `maxPushConstantsSize` minimum every implementation must
    /// support. Exceeding it would not fail here — it would fail at pipeline-layout
    /// creation on whichever machine has the smallest limit, which is exactly the
    /// machine the developer does not have.
    #[test]
    fn the_push_constant_block_fits_the_guaranteed_range() {
        assert!(
            ITEM_BLOCK_SIZE <= 128,
            "the item block is {ITEM_BLOCK_SIZE} bytes; Vulkan only guarantees 128"
        );
        assert_eq!(
            ITEM_BLOCK_SIZE % 16,
            0,
            "the block is a run of ivec4s, so its size must be a multiple of 16"
        );
    }
}
