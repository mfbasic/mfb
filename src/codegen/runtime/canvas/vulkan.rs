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
//! ## The struct layouts are written out, because nothing checks them
//!
//! There is no header to include and no compiler to agree with, so every offset below
//! is the C layout spelled out by hand. A wrong one does not fail to build — it
//! passes garbage to a driver, which is why each struct's field table names its
//! member and its offset, and why the probe is a separate testable step rather than
//! the first half of a pipeline.

use crate::codegen::engine::builder::CodeBuilder;
use crate::codegen::engine::types::*;
use crate::codegen::error::constants::*;
use crate::codegen::link::thunk::emit_data_address;
use crate::codegen::string::util::hex_encode_cstring;
use crate::target::shared::abi;

/// The Vulkan loader's soname on Linux.
pub(crate) const VULKAN_SONAME: &str = "libvulkan.so.1";
/// `RTLD_NOW`; `RTLD_LOCAL` is 0.
const RTLD_NOW: &str = "2";

/// The one entry point resolved by `dlsym`. Everything else comes from it.
const SYM_GET_INSTANCE_PROC_ADDR: &str = "vkGetInstanceProcAddr";
/// Entry points fetched through `vkGetInstanceProcAddr(NULL, …)` — the three that
/// are legal before an instance exists, plus the two the probe needs after one.
const VK_PROBE_ENTRY_POINTS: &[&str] = &[
    "vkCreateInstance",
    "vkEnumeratePhysicalDevices",
    "vkDestroyInstance",
];

/// `VK_STRUCTURE_TYPE_APPLICATION_INFO` / `…_INSTANCE_CREATE_INFO`.
const VK_STRUCTURE_TYPE_APPLICATION_INFO: &str = "0";
const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: &str = "1";
/// `VK_API_VERSION_1_0` — `VK_MAKE_API_VERSION(0, 1, 0, 0)`, i.e. `1 << 22`.
const VK_API_VERSION_1_0: &str = "4194304";

// `VkApplicationInfo`, 48 bytes. The 4-byte `sType` is followed by 4 bytes of
// padding before the 8-byte `pNext`, and likewise after each 4-byte member that
// precedes a pointer — the offsets below are the result, not a guess.
const APP_INFO_SIZE: usize = 48;
const APP_INFO_STYPE: usize = 0;
const APP_INFO_API_VERSION: usize = 44;

// `VkInstanceCreateInfo`, 64 bytes.
const INSTANCE_INFO_SIZE: usize = 64;
const INSTANCE_INFO_STYPE: usize = 0;
const INSTANCE_INFO_APP_INFO: usize = 24;

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
    objects
}

pub(crate) fn soname_symbol() -> String {
    "_mfb_canvas_vk_soname".to_string()
}

pub(crate) fn symbol_name_symbol(name: &str) -> String {
    format!("_mfb_canvas_vk_sym_{name}")
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
    let off_gipa = builder.allocate_stack_object("vk_gipa", 8);
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
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(&unavailable));
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_gipa,
    ));

    // createInstance = gipa(NULL, "vkCreateInstance")
    emit_proc_addr(
        builder,
        "vkCreateInstance",
        None,
        off_gipa,
        off_fn,
        &unavailable,
    );

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
    builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(abi::SCRATCH[1]));
    // VK_SUCCESS is 0; anything else means no instance.
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_ne(&unavailable));

    // enumerate = gipa(instance, "vkEnumeratePhysicalDevices")
    emit_proc_addr(
        builder,
        "vkEnumeratePhysicalDevices",
        Some(off_instance),
        off_gipa,
        off_fn,
        &unavailable,
    );

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
    builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(abi::SCRATCH[1]));

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
    emit_proc_addr(
        builder,
        "vkDestroyInstance",
        Some(off_instance),
        off_gipa,
        off_fn,
        &unavailable,
    );
    builder.emit(abi::load_u64(
        abi::c_arg(0),
        abi::stack_pointer(),
        off_instance,
    ));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "0"));
    builder.emit(abi::load_u64(abi::SCRATCH[1], abi::stack_pointer(), off_fn));
    builder.emit(abi::branch_link_register(abi::SCRATCH[1]));

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

/// `fn = gipa(instance_or_null, "<name>")`, into `off_fn`; branch away if null.
fn emit_proc_addr(
    builder: &mut CodeBuilder,
    name: &str,
    instance_slot: Option<usize>,
    off_gipa: usize,
    off_fn: usize,
    unavailable: &str,
) {
    let symbol = builder.current_symbol.clone();
    match instance_slot {
        Some(slot) => builder.emit(abi::load_u64(abi::c_arg(0), abi::stack_pointer(), slot)),
        None => builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "0")),
    }
    emit_data_address(
        &symbol,
        abi::c_arg(1),
        &symbol_name_symbol(name),
        &mut builder.instructions,
        &mut builder.relocations,
    );
    builder.emit(abi::load_u64(
        abi::SCRATCH[1],
        abi::stack_pointer(),
        off_gipa,
    ));
    builder.emit(abi::branch_link_register(abi::SCRATCH[1]));
    builder.emit(abi::compare_immediate(abi::c_return(0), "0"));
    builder.emit(abi::branch_eq(unavailable));
    builder.emit(abi::store_u64(
        abi::c_return(0),
        abi::stack_pointer(),
        off_fn,
    ));
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
