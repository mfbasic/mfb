//! plan-176-V: the multi-pass frame control — pass branching and the render-pass load. The
//! shared constants, layouts and imports live in `runtime/canvas/vulkan/mod.rs`.

use super::*;

/// bug-765: branch to `target` when `bit` of the pass word parked at `off_pass` is set
/// (`when_set`) or clear. Clobbers `SCRATCH[0]` and `SCRATCH[1]`.
pub(super) fn emit_branch_on_pass(
    builder: &mut CodeBuilder,
    off_pass: usize,
    bit: usize,
    when_set: bool,
    target: &str,
) {
    builder.emit(abi::load_u64(
        abi::SCRATCH[0],
        abi::stack_pointer(),
        off_pass,
    ));
    builder.emit(abi::move_immediate(
        abi::SCRATCH[1],
        "Integer",
        &bit.to_string(),
    ));
    builder.emit(abi::and_registers(
        abi::SCRATCH[0],
        abi::SCRATCH[0],
        abi::SCRATCH[1],
    ));
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    if when_set {
        builder.emit(abi::branch_ne(target));
    } else {
        builder.emit(abi::branch_eq(target));
    }
}

/// bug-765: make sure the render pass a frame's LATER passes record under exists, in
/// the state word `state_slot`: the one colour attachment of every other pass here,
/// LOADED and stored, entered and left in `layout` — the layout the frame's first pass
/// left the image in. It differs from the pass the pipelines were built against only
/// in load operation and layouts, which compatibility does not look at, so nothing is
/// rebuilt. Built once, on the graphics thread, by the first frame of several passes.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_load_render_pass(
    builder: &mut CodeBuilder,
    platform: &dyn CodegenPlatform,
    platform_imports: &std::collections::HashMap<String, String>,
    off_handle: usize,
    off_fn: usize,
    off_state: usize,
    off_out: usize,
    state_slot: usize,
    layout: &str,
    unavailable: &str,
) -> Result<(), String> {
    let have = builder.label("vk_load_pass_have");
    emit_state_load(builder, off_state, state_slot, abi::SCRATCH[0]);
    builder.emit(abi::compare_immediate(abi::SCRATCH[0], "0"));
    builder.emit(abi::branch_ne(&have));
    let off_attachment = builder.allocate_stack_object("vk_load_attachment", ATTACHMENT_SIZE + 4);
    let off_ref = builder.allocate_stack_object("vk_load_attachment_ref", ATTACHMENT_REF_SIZE);
    let off_subpass = builder.allocate_stack_object("vk_load_subpass", SUBPASS_SIZE);
    let off_info = builder.allocate_stack_object("vk_load_pass_info", RENDER_PASS_INFO_SIZE);
    emit_struct(
        builder,
        off_attachment,
        ATTACHMENT_SIZE + 4,
        &[
            (ATTACHMENT_FORMAT, Field::U32(FORMAT_B8G8R8A8_SRGB)),
            (ATTACHMENT_SAMPLES, Field::U32(SAMPLE_COUNT_1)),
            (ATTACHMENT_LOAD_OP, Field::U32(ATTACHMENT_LOAD_OP_LOAD)),
            (ATTACHMENT_STORE_OP, Field::U32(ATTACHMENT_STORE_OP_STORE)),
            (ATTACHMENT_INITIAL_LAYOUT, Field::U32(layout)),
            (ATTACHMENT_FINAL_LAYOUT, Field::U32(layout)),
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
        off_info,
        RENDER_PASS_INFO_SIZE,
        &[
            (RENDER_PASS_STYPE, Field::U32(ST_RENDER_PASS_CREATE_INFO)),
            (RENDER_PASS_ATTACHMENT_COUNT, Field::U32("1")),
            (RENDER_PASS_ATTACHMENTS, Field::Addr(off_attachment)),
            (RENDER_PASS_SUBPASS_COUNT, Field::U32("1")),
            (RENDER_PASS_SUBPASSES, Field::Addr(off_subpass)),
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
        off_info,
        off_out,
        state_slot,
        unavailable,
    )?;
    builder.emit(abi::label(&have));
    Ok(())
}
