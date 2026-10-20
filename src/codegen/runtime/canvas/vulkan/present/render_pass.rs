//! plan-176-V: the present render pass (`emit_present_render_pass`). The shared offsets,
//! `PresentSlots` and the present entry points live in `runtime/canvas/vulkan/present/mod.rs`.

use super::*;

/// The present render pass: the offscreen pass's single sRGB attachment, cleared and
/// stored, ending in `PRESENT_SRC_KHR`, with the external dependency that orders the
/// attachment write after the acquire semaphore's wait. Built once.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_present_render_pass(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    at: &PresentSlots,
    failed: &str,
) -> Result<(), String> {
    let have = builder.label("vk_sc_have_pass");
    emit_state_load(
        builder,
        off_state,
        GRAPHICS_OFFSET_VULKAN_PRESENT_PASS,
        abi::SCRATCH[0],
    );
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(&have));
    let off_attachment = builder.allocate_stack_object("vk_sc_attachment", ATTACHMENT_SIZE + 4);
    let off_ref = builder.allocate_stack_object("vk_sc_attachment_ref", ATTACHMENT_REF_SIZE);
    let off_subpass = builder.allocate_stack_object("vk_sc_subpass", SUBPASS_SIZE);
    let off_dependency = builder.allocate_stack_object("vk_sc_dependency", DEPENDENCY_SIZE);
    emit_struct(
        builder,
        off_attachment,
        ATTACHMENT_SIZE + 4,
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
                Field::U32(IMAGE_LAYOUT_PRESENT_SRC),
            ),
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
        off_dependency,
        DEPENDENCY_SIZE,
        &[
            (DEPENDENCY_SRC_SUBPASS, Field::U32(SUBPASS_EXTERNAL)),
            (DEPENDENCY_DST_SUBPASS, Field::U32("0")),
            (
                DEPENDENCY_SRC_STAGE,
                Field::U32(STAGE_COLOR_ATTACHMENT_OUTPUT),
            ),
            (
                DEPENDENCY_DST_STAGE,
                Field::U32(STAGE_COLOR_ATTACHMENT_OUTPUT),
            ),
            (
                DEPENDENCY_DST_ACCESS,
                Field::U32(ACCESS_COLOR_ATTACHMENT_WRITE),
            ),
        ],
    );
    emit_struct(
        builder,
        at.info,
        RENDER_PASS_INFO_SIZE,
        &[
            (RENDER_PASS_STYPE, Field::U32(ST_RENDER_PASS_CREATE_INFO)),
            (RENDER_PASS_ATTACHMENT_COUNT, Field::U32("1")),
            (RENDER_PASS_ATTACHMENTS, Field::Addr(off_attachment)),
            (RENDER_PASS_SUBPASS_COUNT, Field::U32("1")),
            (RENDER_PASS_SUBPASSES, Field::Addr(off_subpass)),
            (RENDER_PASS_DEPENDENCY_COUNT, Field::U32("1")),
            (RENDER_PASS_DEPENDENCIES, Field::Addr(off_dependency)),
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
        at.info,
        off_out,
        GRAPHICS_OFFSET_VULKAN_PRESENT_PASS,
        failed,
    )?;
    builder.emit(abi::label(&have));
    Ok(())
}
