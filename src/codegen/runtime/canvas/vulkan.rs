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
    push_symbol_address, GRAPHICS_OFFSET_VULKAN_DEVICE, GRAPHICS_OFFSET_VULKAN_INSTANCE,
    GRAPHICS_OFFSET_VULKAN_LIB, GRAPHICS_OFFSET_VULKAN_PHYSICAL, GRAPHICS_OFFSET_VULKAN_PIPELINE,
    GRAPHICS_OFFSET_VULKAN_PIPELINE_LAYOUT, GRAPHICS_OFFSET_VULKAN_QUEUE,
    GRAPHICS_OFFSET_VULKAN_QUEUE_FAMILY, GRAPHICS_OFFSET_VULKAN_READY,
    GRAPHICS_OFFSET_VULKAN_RENDER_PASS, GRAPHICS_STATE_SYMBOL,
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
];

/// `VK_STRUCTURE_TYPE_APPLICATION_INFO` / `…_INSTANCE_CREATE_INFO`.
const VK_STRUCTURE_TYPE_APPLICATION_INFO: &str = "0";
const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: &str = "1";
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

/// The push-constant block both stages read — byte-identical to the Metal item
/// block, so one CPU-side emitter feeds both backends. 112 bytes fits Vulkan's
/// guaranteed 128-byte push-constant range, which is why this path needs no
/// descriptor sets and no buffers.
const ITEM_BLOCK_SIZE: usize = 112;

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
const TOPOLOGY_TRIANGLE_STRIP: &str = "5";
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

/// `canvas::vulkanAvailable() AS Boolean` — can this process render with Vulkan?
///
/// Loads the loader, bootstraps `vkGetInstanceProcAddr`, creates a bare instance
/// (no layers, no extensions — an offscreen renderer needs neither) and asks whether
/// any physical device exists. The instance is destroyed before returning: this
/// answers a question, and the renderer creates and keeps its own.
///
/// Every failure is a plain `FALSE`, never an abort. A machine with no Vulkan loader,
/// no ICD, or no device is a normal machine — it renders in software, which is the
/// default anyway.
///
/// `platform` decides whether any of this is emitted at all: on a target with no
/// `dlopen` the answer is a constant `FALSE`, so the renderer branch keeps one shape
/// everywhere.
///
/// ## Two things this must not do, both learned by segfaulting
///
/// **It takes its scratch from `builder.allocate_stack_object`, never by moving the
/// stack pointer itself.** The enclosing `abi_function` body addresses its own locals
/// off `sp`, so a `sub sp` in the middle of it silently relocates every one of them.
///
/// **It keeps nothing in `abi::LOCAL[…]`.** `%local0` realizes to the arena-state
/// register, which every MFBASIC global and the whole canvas scene region is
/// addressed off. Parking a device count there for the length of one call corrupts
/// the program's entire world, and does it *after* the probe has returned the right
/// answer — so the failure lands somewhere else entirely.
pub(crate) fn emit_vulkan_available(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
) -> Result<(), String> {
    if !matches!(platform.family(), PlatformFamily::Linux) {
        builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));
        return Ok(());
    }
    let symbol = builder.current_symbol.clone();
    let unavailable = builder.label("vk_unavailable");
    let done = builder.label("vk_done");

    // Builder-owned scratch: the two create-info structs, then one word each for the
    // instance handle, the device count, the dlopen handle, `vkGetInstanceProcAddr`,
    // the entry point currently resolved, and the count carried across the teardown.
    let off_app_info = builder.allocate_stack_object("vk_app_info", APP_INFO_SIZE);
    let off_instance_info = builder.allocate_stack_object("vk_instance_info", INSTANCE_INFO_SIZE);
    let off_instance = builder.allocate_stack_object("vk_instance", 8);
    let off_count = builder.allocate_stack_object("vk_count", 8);
    let off_handle = builder.allocate_stack_object("vk_handle", 8);
    let off_fn = builder.allocate_stack_object("vk_fn", 8);
    let off_devices = builder.allocate_stack_object("vk_devices", 8);

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

    // gipa = dlsym(handle, "vkGetInstanceProcAddr")
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_handle,
    ));
    emit_data_address(
        &symbol,
        abi::c_arg(1),
        &symbol_name_symbol(SYM_GET_INSTANCE_PROC_ADDR),
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
    // The result is discarded: this only asks whether the library that answered the
    // `dlopen` is really a Vulkan loader. Every entry point below is resolved from
    // the same handle by name (see `emit_dlsym`), so nothing calls through this.
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&unavailable));

    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkCreateInstance",
        off_handle,
        off_fn,
        &unavailable,
    )?;

    // The two create-info structs. Everything not named here is zero, which is what
    // Vulkan wants for every field this probe does not use — no layers, no
    // extensions, no flags.
    emit_zero_range(builder, off_app_info, APP_INFO_SIZE);
    emit_zero_range(builder, off_instance_info, INSTANCE_INFO_SIZE);
    emit_store_u32_immediate(
        builder,
        off_app_info + APP_INFO_STYPE,
        VK_STRUCTURE_TYPE_APPLICATION_INFO,
    );
    emit_store_u32_immediate(
        builder,
        off_app_info + APP_INFO_API_VERSION,
        VK_API_VERSION_1_0,
    );
    emit_store_u32_immediate(
        builder,
        off_instance_info + INSTANCE_INFO_STYPE,
        VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
    );
    builder.emit(abi::add_immediate(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_app_info,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_instance_info + INSTANCE_INFO_APP_INFO,
    ));

    // result = createInstance(&createInfo, NULL, &instance)
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_instance,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_instance_info,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    builder.emit(abi::add_immediate(
        abi::c_arg(2),
        abi::stack_pointer(),
        off_instance,
    ));
    let callee = builder.temporary_vreg();
    builder.emit(abi::load_u64(&callee, abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(&callee));
    // VK_SUCCESS is 0; anything else means no instance.
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&unavailable));

    // enumerate = gipa(instance, "vkEnumeratePhysicalDevices")
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkEnumeratePhysicalDevices",
        off_handle,
        off_fn,
        &unavailable,
    )?;

    // enumerate(instance, &count, NULL) — the count-only form, which is all the
    // question "is there a device" needs.
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", "0"));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_instance,
    ));
    builder.emit(abi::add_immediate(
        abi::c_arg(1),
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(2), "Integer", "0"));
    let callee = builder.temporary_vreg();
    builder.emit(abi::load_u64(&callee, abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(&callee));

    // Tear the instance down before answering, so asking twice costs a lookup rather
    // than an instance. The count is copied to its own slot first, because
    // `vkDestroyInstance` is free to scribble on the caller's scratch.
    builder.emit(abi::load_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_count,
    ));
    builder.emit(abi::store_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_devices,
    ));
    emit_dlsym(
        builder,
        platform,
        platform_imports,
        "vkDestroyInstance",
        off_handle,
        off_fn,
        &unavailable,
    )?;
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_instance,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    let callee = builder.temporary_vreg();
    builder.emit(abi::load_u64(&callee, abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(&callee));

    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_devices,
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_eq(&unavailable));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&unavailable));
    builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"));

    builder.emit(abi::label(&done));
    Ok(())
}

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
    // VK_INCOMPLETE (5) is fine here — see MAX_PHYSICAL_DEVICES. Only a negative
    // result is a failure, and the count is what decides whether there is a device.
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
    emit_struct(
        builder,
        off_layout_info,
        LAYOUT_INFO_SIZE,
        &[
            (
                LAYOUT_INFO_STYPE,
                Field::U32(ST_PIPELINE_LAYOUT_CREATE_INFO),
            ),
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
    builder.emit(abi::load_u64(
        abi::SCRATCH[3],
        abi::stack_pointer(),
        state_slot,
    ));
    builder.emit(abi::load_u64(dst, abi::SCRATCH[3], offset));
}

/// Call the function pointer parked at `off_fn`.
///
/// Through a **fresh vreg** — see the module doc. Never a fixed `abi::SCRATCH[k]`.
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

fn emit_store_u32_immediate(builder: &mut CodeBuilder, offset: usize, value: &str) {
    builder.emit(abi::move_immediate(abi::SCRATCH[0], "Integer", value));
    builder.emit(abi::store_u32(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        offset,
    ));
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
