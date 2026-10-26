//! plan-176-V: the builder's descriptor set layout, pool and set (`emit_vulkan_descriptors`).
//! The shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

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
pub(super) fn emit_vulkan_descriptors(
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
            // plan-175-B: the vertex stage reads the node region of this buffer too.
            (BINDING_STAGES, Field::U32(SHADER_STAGE_VERTEX_AND_FRAGMENT)),
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
