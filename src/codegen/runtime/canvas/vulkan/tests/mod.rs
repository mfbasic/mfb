use super::*;
use crate::codegen::runtime::canvas::VULKAN_EDGE_BYTES;
use crate::codegen::runtime::canvas::{GRAPHICS_OFFSET_MTL_PIPELINE_MODES, GRAPHICS_STATE_SIZE};

/// Check one hand-transcribed C layout: every `(member, offset, width)` lies inside
/// the struct at its natural alignment, no two members overlap, and the size is a
/// multiple of the struct's alignment. What it cannot check is that the offsets match
/// the Vulkan headers — only a driver can — but a transcription slip (a member moved,
/// a width misread, a pointer at a 4-byte offset) almost always breaks one of these.
pub(super) fn assert_layout(
    name: &str,
    size: usize,
    align: usize,
    members: &[(&str, usize, usize)],
) {
    assert_eq!(
        size % align,
        0,
        "{name}: size {size} is not a multiple of {align}"
    );
    let mut sorted = members.to_vec();
    sorted.sort_by_key(|&(_, offset, _)| offset);
    let mut end = 0;
    for (member, offset, width) in sorted {
        assert!(
            offset + width <= size,
            "{name}.{member}: [{offset}, {}) runs past the {size}-byte struct",
            offset + width
        );
        assert_eq!(
            offset % width.min(8),
            0,
            "{name}.{member}: offset {offset} is not {}-byte aligned",
            width.min(8)
        );
        assert!(
            offset >= end,
            "{name}.{member}: offset {offset} overlaps the member ending at {end}"
        );
        end = offset + width;
    }
}

/// Every struct table above, member by member with its C width — the check the
/// section header promises.
#[test]
fn layout_tables_are_self_consistent() {
    assert_layout(
        "VkApplicationInfo",
        APP_INFO_SIZE,
        8,
        &[
            ("sType", APP_INFO_STYPE, 4),
            ("apiVersion", APP_INFO_API_VERSION, 4),
        ],
    );
    assert_layout(
        "VkInstanceCreateInfo",
        INSTANCE_INFO_SIZE,
        8,
        &[
            ("sType", INSTANCE_INFO_STYPE, 4),
            ("pApplicationInfo", INSTANCE_INFO_APP_INFO, 8),
        ],
    );
    assert_layout(
        "VkDeviceQueueCreateInfo",
        QUEUE_INFO_SIZE,
        8,
        &[
            ("sType", QUEUE_INFO_STYPE, 4),
            ("queueFamilyIndex", QUEUE_INFO_FAMILY, 4),
            ("queueCount", QUEUE_INFO_COUNT, 4),
            ("pQueuePriorities", QUEUE_INFO_PRIORITIES, 8),
        ],
    );
    assert_layout(
        "VkDeviceCreateInfo",
        DEVICE_INFO_SIZE,
        8,
        &[
            ("sType", DEVICE_INFO_STYPE, 4),
            ("queueCreateInfoCount", DEVICE_INFO_QUEUE_COUNT, 4),
            ("pQueueCreateInfos", DEVICE_INFO_QUEUE_INFOS, 8),
        ],
    );
    assert_layout(
        "VkQueueFamilyProperties",
        QUEUE_FAMILY_PROPERTIES_SIZE,
        4,
        &[("queueFlags", QUEUE_FAMILY_FLAGS, 4)],
    );
    assert_layout(
        "VkShaderModuleCreateInfo",
        SHADER_MODULE_INFO_SIZE,
        8,
        &[
            ("sType", SHADER_MODULE_STYPE, 4),
            ("codeSize", SHADER_MODULE_CODE_SIZE, 8),
            ("pCode", SHADER_MODULE_CODE, 8),
        ],
    );
    assert_layout(
        "VkPipelineLayoutCreateInfo",
        LAYOUT_INFO_SIZE,
        8,
        &[
            ("sType", LAYOUT_INFO_STYPE, 4),
            ("setLayoutCount", LAYOUT_INFO_SET_COUNT, 4),
            ("pSetLayouts", LAYOUT_INFO_SETS, 8),
            ("pushConstantRangeCount", LAYOUT_INFO_RANGE_COUNT, 4),
            ("pPushConstantRanges", LAYOUT_INFO_RANGES, 8),
        ],
    );
    assert_layout(
        "VkPushConstantRange",
        PUSH_RANGE_SIZE,
        4,
        &[
            ("stageFlags", PUSH_RANGE_STAGES, 4),
            ("offset", PUSH_RANGE_OFFSET, 4),
            ("size", PUSH_RANGE_BYTES, 4),
        ],
    );
    assert_layout(
        "VkAttachmentDescription",
        ATTACHMENT_SIZE,
        4,
        &[
            ("format", ATTACHMENT_FORMAT, 4),
            ("samples", ATTACHMENT_SAMPLES, 4),
            ("loadOp", ATTACHMENT_LOAD_OP, 4),
            ("storeOp", ATTACHMENT_STORE_OP, 4),
            ("initialLayout", ATTACHMENT_INITIAL_LAYOUT, 4),
            ("finalLayout", ATTACHMENT_FINAL_LAYOUT, 4),
        ],
    );
    assert_layout(
        "VkAttachmentReference",
        ATTACHMENT_REF_SIZE,
        4,
        &[
            ("attachment", ATTACHMENT_REF_INDEX, 4),
            ("layout", ATTACHMENT_REF_LAYOUT, 4),
        ],
    );
    assert_layout(
        "VkSubpassDescription",
        SUBPASS_SIZE,
        8,
        &[
            ("pipelineBindPoint", SUBPASS_BIND_POINT, 4),
            ("colorAttachmentCount", SUBPASS_COLOR_COUNT, 4),
            ("pColorAttachments", SUBPASS_COLOR_ATTACHMENTS, 8),
        ],
    );
    assert_layout(
        "VkRenderPassCreateInfo",
        RENDER_PASS_INFO_SIZE,
        8,
        &[
            ("sType", RENDER_PASS_STYPE, 4),
            ("attachmentCount", RENDER_PASS_ATTACHMENT_COUNT, 4),
            ("pAttachments", RENDER_PASS_ATTACHMENTS, 8),
            ("subpassCount", RENDER_PASS_SUBPASS_COUNT, 4),
            ("pSubpasses", RENDER_PASS_SUBPASSES, 8),
        ],
    );
    assert_layout(
        "VkPipelineShaderStageCreateInfo",
        STAGE_INFO_SIZE,
        8,
        &[
            ("sType", STAGE_INFO_STYPE, 4),
            ("stage", STAGE_INFO_STAGE, 4),
            ("module", STAGE_INFO_MODULE, 8),
            ("pName", STAGE_INFO_NAME, 8),
            ("pSpecializationInfo", STAGE_INFO_SPECIALIZATION, 8),
        ],
    );
    assert_layout(
        "VkSpecializationInfo",
        SPEC_INFO_SIZE,
        8,
        &[
            ("mapEntryCount", SPEC_INFO_COUNT, 4),
            ("pMapEntries", SPEC_INFO_ENTRIES, 8),
            ("dataSize", SPEC_INFO_DATA_SIZE, 8),
            ("pData", SPEC_INFO_DATA, 8),
        ],
    );
    assert_layout(
        "VkSpecializationMapEntry",
        SPEC_ENTRY_SIZE,
        8,
        &[
            ("constantID", SPEC_ENTRY_ID, 4),
            ("offset", SPEC_ENTRY_OFFSET, 4),
            ("size", SPEC_ENTRY_BYTES, 8),
        ],
    );
    assert_layout(
        "VkPipelineVertexInputStateCreateInfo",
        VERTEX_INPUT_INFO_SIZE,
        8,
        &[],
    );
    assert_layout(
        "VkPipelineInputAssemblyStateCreateInfo",
        INPUT_ASSEMBLY_INFO_SIZE,
        8,
        &[("topology", INPUT_ASSEMBLY_TOPOLOGY, 4)],
    );
    assert_layout(
        "VkPipelineViewportStateCreateInfo",
        VIEWPORT_INFO_SIZE,
        8,
        &[
            ("viewportCount", VIEWPORT_INFO_VIEWPORT_COUNT, 4),
            ("scissorCount", VIEWPORT_INFO_SCISSOR_COUNT, 4),
        ],
    );
    assert_layout(
        "VkPipelineRasterizationStateCreateInfo",
        RASTER_INFO_SIZE,
        8,
        &[
            ("polygonMode", RASTER_POLYGON_MODE, 4),
            ("cullMode", RASTER_CULL_MODE, 4),
            ("frontFace", RASTER_FRONT_FACE, 4),
            ("lineWidth", RASTER_LINE_WIDTH, 4),
        ],
    );
    assert_layout(
        "VkPipelineMultisampleStateCreateInfo",
        MULTISAMPLE_INFO_SIZE,
        8,
        &[("rasterizationSamples", MULTISAMPLE_SAMPLES, 4)],
    );
    assert_layout(
        "VkPipelineColorBlendAttachmentState",
        BLEND_ATTACHMENT_SIZE,
        4,
        &[
            ("blendEnable", BLEND_ENABLE, 4),
            ("srcColorBlendFactor", BLEND_SRC_COLOR, 4),
            ("dstColorBlendFactor", BLEND_DST_COLOR, 4),
            ("colorBlendOp", BLEND_COLOR_OP, 4),
            ("srcAlphaBlendFactor", BLEND_SRC_ALPHA, 4),
            ("dstAlphaBlendFactor", BLEND_DST_ALPHA, 4),
            ("alphaBlendOp", BLEND_ALPHA_OP, 4),
            ("colorWriteMask", BLEND_WRITE_MASK, 4),
        ],
    );
    assert_layout(
        "VkPipelineColorBlendStateCreateInfo",
        BLEND_INFO_SIZE,
        8,
        &[
            ("attachmentCount", BLEND_INFO_ATTACHMENT_COUNT, 4),
            ("pAttachments", BLEND_INFO_ATTACHMENTS, 8),
        ],
    );
    assert_layout(
        "VkPipelineDynamicStateCreateInfo",
        DYNAMIC_INFO_SIZE,
        8,
        &[
            ("dynamicStateCount", DYNAMIC_INFO_COUNT, 4),
            ("pDynamicStates", DYNAMIC_INFO_STATES, 8),
        ],
    );
    assert_layout(
        "VkGraphicsPipelineCreateInfo",
        PIPELINE_INFO_SIZE,
        8,
        &[
            ("stageCount", PIPELINE_STAGE_COUNT, 4),
            ("pStages", PIPELINE_STAGES, 8),
            ("pVertexInputState", PIPELINE_VERTEX_INPUT, 8),
            ("pInputAssemblyState", PIPELINE_INPUT_ASSEMBLY, 8),
            ("pViewportState", PIPELINE_VIEWPORT, 8),
            ("pRasterizationState", PIPELINE_RASTER, 8),
            ("pMultisampleState", PIPELINE_MULTISAMPLE, 8),
            ("pColorBlendState", PIPELINE_COLOR_BLEND, 8),
            ("pDynamicState", PIPELINE_DYNAMIC, 8),
            ("layout", PIPELINE_LAYOUT, 8),
            ("renderPass", PIPELINE_RENDER_PASS, 8),
        ],
    );
    assert_layout(
        "VkImageCreateInfo",
        IMAGE_INFO_SIZE,
        8,
        &[
            ("imageType", IMAGE_INFO_TYPE, 4),
            ("format", IMAGE_INFO_FORMAT, 4),
            ("extent.width", IMAGE_INFO_WIDTH, 4),
            ("extent.height", IMAGE_INFO_HEIGHT, 4),
            ("extent.depth", IMAGE_INFO_DEPTH, 4),
            ("mipLevels", IMAGE_INFO_MIP_LEVELS, 4),
            ("arrayLayers", IMAGE_INFO_ARRAY_LAYERS, 4),
            ("samples", IMAGE_INFO_SAMPLES, 4),
            ("tiling", IMAGE_INFO_TILING, 4),
            ("usage", IMAGE_INFO_USAGE, 4),
            ("initialLayout", IMAGE_INFO_INITIAL_LAYOUT, 4),
        ],
    );
    assert_layout(
        "VkMemoryRequirements",
        MEMORY_REQS_SIZE,
        8,
        &[
            ("size", MEMORY_REQS_BYTES, 8),
            ("memoryTypeBits", MEMORY_REQS_TYPE_BITS, 4),
        ],
    );
    assert_layout(
        "VkMemoryAllocateInfo",
        ALLOC_INFO_SIZE,
        8,
        &[
            ("allocationSize", ALLOC_INFO_BYTES, 8),
            ("memoryTypeIndex", ALLOC_INFO_TYPE_INDEX, 4),
        ],
    );
    assert_layout(
        "VkPhysicalDeviceMemoryProperties",
        MEMORY_PROPERTIES_SIZE,
        8,
        &[
            ("memoryTypeCount", MEMORY_PROPERTIES_TYPE_COUNT, 4),
            // `VkMemoryType[32]`, two `u32`s each: its first and last members.
            ("memoryTypes[0].propertyFlags", MEMORY_PROPERTIES_TYPES, 4),
            (
                "memoryTypes[31].heapIndex",
                MEMORY_PROPERTIES_TYPES + 31 * MEMORY_TYPE_STRIDE + 4,
                4,
            ),
        ],
    );
    assert_layout(
        "VkImageViewCreateInfo",
        VIEW_INFO_SIZE,
        8,
        &[
            ("image", VIEW_INFO_IMAGE, 8),
            ("viewType", VIEW_INFO_TYPE, 4),
            ("format", VIEW_INFO_FORMAT, 4),
            ("subresourceRange.aspectMask", VIEW_INFO_ASPECT, 4),
            ("subresourceRange.levelCount", VIEW_INFO_LEVEL_COUNT, 4),
            ("subresourceRange.layerCount", VIEW_INFO_LAYER_COUNT, 4),
        ],
    );
    assert_layout(
        "VkFramebufferCreateInfo",
        FRAMEBUFFER_INFO_SIZE,
        8,
        &[
            ("renderPass", FRAMEBUFFER_RENDER_PASS, 8),
            ("attachmentCount", FRAMEBUFFER_ATTACHMENT_COUNT, 4),
            ("pAttachments", FRAMEBUFFER_ATTACHMENTS, 8),
            ("width", FRAMEBUFFER_WIDTH, 4),
            ("height", FRAMEBUFFER_HEIGHT, 4),
            ("layers", FRAMEBUFFER_LAYERS, 4),
        ],
    );
    assert_layout(
        "VkBufferCreateInfo",
        BUFFER_INFO_SIZE,
        8,
        &[
            ("size", BUFFER_INFO_BYTES, 8),
            ("usage", BUFFER_INFO_USAGE, 4),
        ],
    );
    assert_layout(
        "VkCommandPoolCreateInfo",
        POOL_INFO_SIZE,
        8,
        &[
            ("flags", POOL_INFO_FLAGS, 4),
            ("queueFamilyIndex", POOL_INFO_FAMILY, 4),
        ],
    );
    assert_layout(
        "VkCommandBufferAllocateInfo",
        CMD_ALLOC_INFO_SIZE,
        8,
        &[
            ("commandPool", CMD_ALLOC_POOL, 8),
            ("level", CMD_ALLOC_LEVEL, 4),
            ("commandBufferCount", CMD_ALLOC_COUNT, 4),
        ],
    );
    assert_layout(
        "VkCommandBufferBeginInfo",
        CMD_BEGIN_INFO_SIZE,
        8,
        &[("flags", CMD_BEGIN_FLAGS, 4)],
    );
    assert_layout(
        "VkRenderPassBeginInfo",
        PASS_BEGIN_INFO_SIZE,
        8,
        &[
            ("renderPass", PASS_BEGIN_RENDER_PASS, 8),
            ("framebuffer", PASS_BEGIN_FRAMEBUFFER, 8),
            ("renderArea.extent.width", PASS_BEGIN_AREA_WIDTH, 4),
            ("renderArea.extent.height", PASS_BEGIN_AREA_HEIGHT, 4),
            ("clearValueCount", PASS_BEGIN_CLEAR_COUNT, 4),
            ("pClearValues", PASS_BEGIN_CLEARS, 8),
        ],
    );
    assert_layout(
        "VkClearValue",
        CLEAR_VALUE_SIZE,
        4,
        &[("color.float32[3]", CLEAR_ALPHA, 4)],
    );
    assert_layout(
        "VkViewport",
        VIEWPORT_SIZE,
        4,
        &[
            ("width", VIEWPORT_WIDTH, 4),
            ("height", VIEWPORT_HEIGHT, 4),
            ("maxDepth", VIEWPORT_MAX_DEPTH, 4),
        ],
    );
    assert_layout(
        "VkRect2D",
        RECT_SIZE,
        4,
        &[
            ("extent.width", RECT_WIDTH, 4),
            ("extent.height", RECT_HEIGHT, 4),
        ],
    );
    assert_layout(
        "VkBufferImageCopy",
        COPY_SIZE,
        8,
        &[
            ("imageSubresource.aspectMask", COPY_ASPECT, 4),
            ("imageSubresource.layerCount", COPY_LAYER_COUNT, 4),
            ("imageExtent.width", COPY_EXTENT_WIDTH, 4),
            ("imageExtent.height", COPY_EXTENT_HEIGHT, 4),
            ("imageExtent.depth", COPY_EXTENT_DEPTH, 4),
        ],
    );
    assert_layout(
        "VkSubmitInfo",
        SUBMIT_INFO_SIZE,
        8,
        &[
            ("commandBufferCount", SUBMIT_COMMAND_COUNT, 4),
            ("pCommandBuffers", SUBMIT_COMMANDS, 8),
        ],
    );
    assert_layout(
        "VkDescriptorSetLayoutBinding",
        BINDING_SIZE,
        8,
        &[
            ("binding", BINDING_INDEX, 4),
            ("descriptorType", BINDING_TYPE, 4),
            ("descriptorCount", BINDING_COUNT, 4),
            ("stageFlags", BINDING_STAGES, 4),
        ],
    );
    assert_layout(
        "VkDescriptorSetLayoutCreateInfo",
        SET_LAYOUT_INFO_SIZE,
        8,
        &[
            ("bindingCount", SET_LAYOUT_BINDING_COUNT, 4),
            ("pBindings", SET_LAYOUT_BINDINGS, 8),
        ],
    );
    assert_layout(
        "VkDescriptorPoolSize",
        POOL_SIZE_SIZE,
        4,
        &[
            ("type", POOL_SIZE_TYPE, 4),
            ("descriptorCount", POOL_SIZE_COUNT, 4),
        ],
    );
    assert_layout(
        "VkDescriptorPoolCreateInfo",
        DESC_POOL_INFO_SIZE,
        8,
        &[
            ("maxSets", DESC_POOL_MAX_SETS, 4),
            ("poolSizeCount", DESC_POOL_SIZE_COUNT, 4),
            ("pPoolSizes", DESC_POOL_SIZES, 8),
        ],
    );
    assert_layout(
        "VkDescriptorSetAllocateInfo",
        DESC_ALLOC_INFO_SIZE,
        8,
        &[
            ("descriptorPool", DESC_ALLOC_POOL, 8),
            ("descriptorSetCount", DESC_ALLOC_SET_COUNT, 4),
            ("pSetLayouts", DESC_ALLOC_LAYOUTS, 8),
        ],
    );
    assert_layout(
        "VkDescriptorBufferInfo",
        DESC_BUFFER_INFO_SIZE,
        8,
        &[
            ("buffer", DESC_BUFFER_INFO_BUFFER, 8),
            ("range", DESC_BUFFER_INFO_RANGE, 8),
        ],
    );
    assert_layout(
        "VkWriteDescriptorSet",
        WRITE_SET_SIZE,
        8,
        &[
            ("dstSet", WRITE_SET_DST, 8),
            ("dstBinding", WRITE_SET_BINDING, 4),
            ("descriptorCount", WRITE_SET_COUNT, 4),
            ("descriptorType", WRITE_SET_TYPE, 4),
            ("pBufferInfo", WRITE_SET_BUFFER_INFO, 8),
        ],
    );
}

/// bug-688 V5: the readback buffer is read by the CPU every frame, so it asks for
/// a `HOST_CACHED` type first and only then the plain coherent one. On box 206's
/// Intel UHD 630 the first coherent type is write-combined (uncached reads) and
/// took ~200 ms a frame to read back; the cached type the same driver offers cut
/// the 95-item frame's render phase to ~13 ms.
#[test]
fn the_readback_buffer_prefers_cached_memory() {
    let flags: Vec<u32> = READBACK_MEMORY_CANDIDATES
        .iter()
        .map(|f| f.parse().expect("numeric flags"))
        .collect();
    const HOST_VISIBLE: u32 = 0x2;
    const HOST_COHERENT: u32 = 0x4;
    const HOST_CACHED: u32 = 0x8;
    assert_eq!(
        flags[0],
        HOST_VISIBLE | HOST_COHERENT | HOST_CACHED,
        "the first candidate must be the cached, coherent, host-visible type"
    );
    assert_eq!(
        *flags.last().unwrap(),
        HOST_VISIBLE | HOST_COHERENT,
        "the last candidate must be the coherent type every device offers"
    );
    assert!(
        flags
            .iter()
            .all(|f| f & (HOST_VISIBLE | HOST_COHERENT) == HOST_VISIBLE | HOST_COHERENT),
        "every candidate must stay coherent: the copy is read with no invalidate"
    );
}

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

/// The two backends' pipeline arrays hold one entry per `BlendMode`, and the two
/// arrays do not overlap each other or run past the state block.
///
/// The blend tag reaches the frame path from `HEADER_BLEND` as a plain 0..3 and is
/// used as an *index* with no bounds check — `base + mode * 8` — so an array
/// shorter than the variant set reads a neighbouring state slot and binds it as a
/// pipeline handle. Nothing catches that at run time: the value is a pointer either
/// way, and the frame comes back wrong rather than failing.
#[test]
fn each_backend_has_one_pipeline_slot_per_blend_mode() {
    assert_eq!(
        BLEND_MODE_COUNT, 4,
        "BlendMode has four variants (Normal, Multiply, Screen, Add); both pipeline \
             arrays are sized from this"
    );
    assert_eq!(
        GRAPHICS_OFFSET_MTL_PIPELINE_MODES,
        GRAPHICS_OFFSET_VULKAN_PIPELINE_MODES + BLEND_MODE_COUNT * 8,
        "the Metal array must start where the Vulkan one ends: a gap wastes state, \
             and an overlap makes one backend bind the other's handles"
    );
    assert!(
        GRAPHICS_OFFSET_MTL_PIPELINE_MODES + BLEND_MODE_COUNT * 8 <= GRAPHICS_STATE_SIZE,
        "the pipeline arrays run past the end of the graphics state block"
    );
}

/// plan-172-J: the pointer's five words are 40 contiguous bytes, the sequence word
/// first and the four published words after it in the order
/// `canvas::pointerSnapshot` returns them. Three platform handlers and one reader
/// address them by constant.
///
/// plan-172-K put two words after them — `AUTOPAINT_LIVE`, then `AUTO_WAKE` — so
/// the pointer's are no longer the block's last; the block is sized from the new
/// last word, and that is what the first two assertions now say. Written as "the
/// pointer words end the block" (plan-172-J), which was true of the block then and
/// was never what the handlers depend on: they depend on the five being contiguous
/// and in order, which the loop still checks.
#[test]
fn the_pointer_words_end_the_graphics_state() {
    use crate::codegen::runtime::canvas::{
        GRAPHICS_OFFSET_AUTOPAINT_LIVE, GRAPHICS_OFFSET_AUTO_WAKE, GRAPHICS_OFFSET_POINTER_INSIDE,
        GRAPHICS_OFFSET_POINTER_SEQ, GRAPHICS_POINTER_WORDS,
    };
    assert_eq!(
        GRAPHICS_OFFSET_AUTOPAINT_LIVE,
        GRAPHICS_OFFSET_POINTER_SEQ + 40
    );
    assert_eq!(
        GRAPHICS_OFFSET_POINTER_INSIDE + 8,
        GRAPHICS_OFFSET_AUTOPAINT_LIVE
    );
    assert_eq!(
        GRAPHICS_OFFSET_AUTO_WAKE,
        GRAPHICS_OFFSET_AUTOPAINT_LIVE + 8
    );
    // And the baked `options.canvasAutoPaintMS`, the block's last word and the only
    // one that does not start as zero.
    use crate::codegen::runtime::canvas::{
        graphics_state_data_object, GRAPHICS_OFFSET_AUTO_WINDOW_MS,
    };
    assert_eq!(
        GRAPHICS_OFFSET_AUTO_WINDOW_MS,
        GRAPHICS_OFFSET_AUTO_WAKE + 8
    );
    assert_eq!(GRAPHICS_STATE_SIZE, GRAPHICS_OFFSET_AUTO_WINDOW_MS + 8);
    let block = graphics_state_data_object(0x0102);
    assert_eq!(block.value.len(), GRAPHICS_STATE_SIZE * 2);
    let at = GRAPHICS_OFFSET_AUTO_WINDOW_MS * 2;
    assert_eq!(&block.value[at..at + 16], "0201000000000000");
    assert!(block.value[..at].bytes().all(|digit| digit == b'0'));
    for (index, offset) in GRAPHICS_POINTER_WORDS.into_iter().enumerate() {
        assert_eq!(offset, GRAPHICS_OFFSET_POINTER_SEQ + 8 * (index + 1));
    }
    assert_eq!(GRAPHICS_OFFSET_POINTER_SEQ % 8, 0);
}

/// The item block is the size the GLSL `ItemBlock`'s std430 array stride is.
///
/// **This replaced a check against Vulkan's guaranteed 128-byte push-constant
/// range** (plan-116-A). That bound was real while the block *was* a push constant:
/// 128 is the `maxPushConstantsSize` minimum every implementation must support, and
/// exceeding it would have failed at pipeline-layout creation on whichever machine
/// had the smallest limit — which is exactly the machine the developer does not
/// have. The block now rides a storage buffer, so that ceiling is gone, and
/// asserting it would pin a constraint nothing enforces.
///
/// What replaces it is the constraint that *is* still live: the CPU emitter writes
/// records of `ITEM_BLOCK_SIZE` bytes and the shaders index an `ItemBlock[]`, so the
/// two agree only if std430's array stride equals that size. std430 gives a struct
/// the alignment of its largest member — `ivec4`, 16 bytes — and rounds the stride
/// up to a multiple of it. So the stride equals the size exactly when the size is a
/// multiple of 16, which is what this asserts, and it is why every member of the
/// block is an `ivec4` rather than packed.
///
/// Measured against the real compiler rather than reasoned about alone:
/// `glslangValidator -V -q mfb_canvas.vert` reports `topLevelArrayStride 160` with
/// members at 0/16/32/48/64/80/96/112/128/144 (2026-09-01, glslang 11:15.2.0) —
/// re-measured each time the block grew: 112 → 128 for plan-116-B's clip, then
/// 128 → 160 for plan-116-C's transform. A later letter that widens it again must
/// re-run that and keep this equality.
#[test]
fn the_item_block_matches_the_std430_stride() {
    assert_eq!(
        ITEM_BLOCK_SIZE % 16,
        0,
        "the block is a run of ivec4s: std430 rounds its array stride up to a \
             multiple of 16, so a size that is not one makes the stride differ from the \
             {ITEM_BLOCK_SIZE} bytes the emitter writes per record, and every item after \
             the first would read a shifted block"
    );
    assert_eq!(
        VULKAN_ITEM_BUFFER_BYTES,
        VULKAN_MAX_FRAME_ITEMS * ITEM_BLOCK_SIZE,
        "the buffer must hold exactly the number of records the predicates admit"
    );
}

/// The fragment shader's region bases are specialization constants, and the
/// pipeline feeds every one of them (bug-688).
///
/// This replaced two tests that compared literals in the GLSL (`const int GLYPH_BASE
/// = 65536;`, `const int GRADIENT_BASE = 1114112;`) against the Rust layout. Those
/// literals are gone: each base is now `layout(constant_id = N) const int` with a
/// default of 0, and its value arrives through `VkSpecializationInfo` from
/// `VULKAN_SPEC_CONSTANTS`, which is built from the layout constants themselves. So
/// what can still disagree is the NAME-to-ID pairing — the GLSL says `GLYPH_BASE` is
/// id 0 and the Rust table must send the glyph base as id 0 — and whether the blob
/// was regenerated from this GLSL at all. A wrong pairing, or a stale blob with the
/// literals, does not fail anywhere: every glyph, ramp or band reads from another
/// region of a buffer that is entirely valid memory.
#[test]
fn the_pipeline_specializes_every_region_base() {
    const GLSL: &str = include_str!("../../shaders/mfb_canvas.frag");
    let expected = [
        ("GLYPH_BASE", VULKAN_GLYPH_BASE_WORDS),
        ("GRADIENT_BASE", VULKAN_GRADIENT_BASE_WORDS),
        ("BAND_BASE", VULKAN_BAND_BASE_WORDS),
        // plan-163-B: the dash words.
        ("DASH_BASE", VULKAN_DASH_BASE_WORDS),
        // plan-175-B: the node table, which the vertex stage reads too (below).
        (
            "NODE_BASE",
            crate::codegen::runtime::canvas::VULKAN_NODE_BASE_WORDS,
        ),
    ];
    assert_eq!(
        VULKAN_SPEC_CONSTANTS.len(),
        expected.len(),
        "one specialization entry per region base the shader reads"
    );
    for (name, words) in expected {
        let line = GLSL
            .lines()
            .find(|l| l.contains(&format!("const int {name} ")))
            .unwrap_or_else(|| panic!("the fragment shader does not declare {name}"));
        let id: u32 = line
            .split("constant_id")
            .nth(1)
            .and_then(|rest| rest.trim_start_matches([' ', '=']).split(')').next())
            .and_then(|n| n.trim().parse().ok())
            .unwrap_or_else(|| {
                panic!("{name} is not a specialization constant in the GLSL: `{line}`")
            });
        assert!(
            VULKAN_SPEC_CONSTANTS.contains(&(id, words)),
            "the GLSL reads {name} as constant_id {id}, but the pipeline does not send \
                 its base ({words} words) under that id: {VULKAN_SPEC_CONSTANTS:?}"
        );
    }
    // The blob is compiled from that GLSL: each id the shader reads is decorated
    // `SpecId` (OpDecorate = 71, SpecId = 1) in the checked-in SPIR-V. A blob still
    // holding the literals has no such decoration.
    let spec_ids_of = |blob: &[u8]| -> Vec<u32> {
        let words: Vec<u32> = blob
            .chunks_exact(4)
            .map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
            .collect();
        let mut spec_ids = Vec::new();
        let mut at = 5;
        while at < words.len() {
            let (count, opcode) = ((words[at] >> 16) as usize, words[at] & 0xffff);
            if opcode == 71 && count == 4 && words[at + 2] == 1 {
                spec_ids.push(words[at + 3]);
            }
            at += count.max(1);
        }
        spec_ids
    };
    let spec_ids = spec_ids_of(SPIRV_FRAGMENT);
    for (id, _) in VULKAN_SPEC_CONSTANTS {
        assert!(
            spec_ids.contains(&id),
            "the fragment SPIR-V has no SpecId {id} — it was not regenerated from the \
                 GLSL (scripts/regen-spirv.sh). Found: {spec_ids:?}"
        );
    }
    // plan-175-B: the vertex stage reads the node table, so its blob declares id 5 too
    // (`layout(constant_id = 5) const int NODE_BASE` in `mfb_canvas.vert`).
    let vertex_ids = spec_ids_of(SPIRV_VERTEX);
    assert!(
        vertex_ids.contains(&5),
        "the vertex SPIR-V has no SpecId 5 (NODE_BASE): {vertex_ids:?}"
    );
}

/// The shared buffer is exactly the regions it is asked to hold, and each region
/// starts where the one before it ends.
///
/// Written as a running total rather than as a sum, so adding a region means
/// extending the chain rather than rewriting an equation — plan-116-F added the
/// third and found this asserting the two-region total. bug-688 added the band index
/// and the picture table and reordered the regions to Metal's order; the bases the
/// shader reads are specialized from these constants, so the chain is the whole
/// layout contract.
#[test]
fn the_shared_buffer_holds_every_region() {
    let mut end = VULKAN_EDGE_BYTES;
    assert_eq!(
        VULKAN_BAND_BASE_WORDS * 4,
        end,
        "the band region must start where the edge region ends",
    );
    end += VULKAN_MAX_FRAME_BAND_WORDS * 4;
    assert_eq!(
        VULKAN_GRADIENT_BASE_WORDS * 4,
        end,
        "the gradient region must start where the band region ends",
    );
    end += VULKAN_MAX_FRAME_GRADIENT_STOPS * GRADIENT_STOP_WORDS * 4;
    // plan-163-B put the dash words between the gradient stops and the glyphs.
    assert_eq!(
        VULKAN_DASH_BASE_WORDS * 4,
        end,
        "the dash region must start where the gradient region ends",
    );
    end += VULKAN_MAX_FRAME_DASH_WORDS * 4;
    assert_eq!(
        VULKAN_GLYPH_BASE_WORDS * 4,
        end,
        "the glyph region must start where the dash region ends",
    );
    end += VULKAN_MAX_FRAME_GLYPH_SAMPLES * 4;
    assert_eq!(
        VULKAN_PICTURE_RECORDS_OFFSET, end,
        "the picture records must start where the glyph region ends",
    );
    end += VULKAN_PICTURE_RECORDS * PICTURE_RECORD_BYTES;
    assert_eq!(
        VULKAN_PICTURE_INDEX_OFFSET, end,
        "the picture index must start where the records end",
    );
    end += VULKAN_PICTURE_INDEX_SLOTS * 4;
    // plan-175-B: the node table, last.
    assert_eq!(
        crate::codegen::runtime::canvas::VULKAN_NODE_BASE_WORDS * 4,
        end,
        "the node table must start where the picture index ends",
    );
    end += crate::codegen::runtime::canvas::VULKAN_MAX_FRAME_NODES
        * crate::codegen::runtime::canvas::NODE_BYTES;
    assert_eq!(
        VULKAN_BUFFER_BYTES, end,
        "the buffer must be exactly its regions, with nothing past the last",
    );
    // Every region base the shader indexes is an `int`, and so is every index it
    // computes inside the glyph region.
    assert!(
        VULKAN_PICTURE_RECORDS_OFFSET / 4 < i32::MAX as usize,
        "a word index past i32::MAX cannot be read by the GLSL's `int` arithmetic"
    );
}
