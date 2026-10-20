//! plan-176-V: the WSI instance and device creation (`emit_wsi_create_instance`,
//! `emit_wsi_create_device`). The shared offsets, `PresentSlots` and the present entry points
//! live in `runtime/canvas/vulkan/present/mod.rs`.

use super::*;

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
pub(in crate::codegen::runtime::canvas::vulkan) fn emit_wsi_create_instance(
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
pub(in crate::codegen::runtime::canvas::vulkan) fn emit_wsi_create_device(
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
