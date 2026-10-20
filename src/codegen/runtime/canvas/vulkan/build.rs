//! plan-176-V: the builder thread's entry (`emit_vulkan_build`), its state publication and
//! spawn, and the ready hand-off (`emit_vulkan_ready`). The shared constants, layouts and
//! imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

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
