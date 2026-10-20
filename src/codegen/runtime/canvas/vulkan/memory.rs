//! plan-176-V: Vulkan device-memory allocation and binding, and the memory-type scan. The
//! shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// Allocate memory for the requirements at `off_reqs` and bind it to `object`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_allocate_and_bind(
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
