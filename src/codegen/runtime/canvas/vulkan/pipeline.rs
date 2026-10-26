//! plan-176-V: the builder's render pass and graphics pipelines (`emit_vulkan_pipeline`). The
//! shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

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
pub(super) fn emit_vulkan_pipeline(
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
    // plan-175-B: one push-constant range, the draw's RUN, consumed by the vertex stage
    // only — the fragment stage reads its node by index. `emit_struct` zeroes the struct
    // first, so the fields not named here are already 0 — which is what `offset` must
    // be, this being the only range.
    let off_push_range = builder.allocate_stack_object("vk_push_range", PUSH_RANGE_SIZE);
    emit_struct(
        builder,
        off_push_range,
        PUSH_RANGE_SIZE,
        &[
            (PUSH_RANGE_STAGES, Field::U32(SHADER_STAGE_VERTEX)),
            (PUSH_RANGE_OFFSET, Field::U32("0")),
            (PUSH_RANGE_BYTES, Field::U32(&PUSH_RUN_BYTES.to_string())),
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
        // plan-175-B: both stages are specialized — the vertex stage reads the node
        // region's base (constant 5) as the fragment stage does, and ignores the rest.
        emit_struct(
            builder,
            base + STAGE_INFO_SPECIALIZATION,
            8,
            &[(0, Field::Addr(off_spec_info))],
        );
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
