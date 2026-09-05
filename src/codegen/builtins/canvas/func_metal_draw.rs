//! `canvas::metalDrawScene` — hand one frame's geometry to the Metal renderer.
//!
//! Internal-only, and the counterpart to `canvas::blitSurface`: that one hands a
//! finished frame *out* to the platform, this one asks the platform to produce one.
//! Both take the surface's payload pointer rather than the collection, and for the
//! same reason — a 2.3 MB frame is not something to copy per call.
//!
//! It writes **through** the surface argument instead of returning a new one. The
//! buffer comes straight from `canvas::newSurface` inside `__canvas_renderMetal` and
//! is not aliased by anything, so an in-place write is safe there; returning the same
//! block from an MFBASIC call would mean assigning a collection to itself, which is
//! the one shape the ownership model has no answer for.

use crate::codegen::collection::layout::list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::runtime::canvas::vulkan::emit_vulkan_draw_scene;
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `canvas::metalDrawScene(surface, width, height, geometry, offsets) AS Nothing`.
pub(crate) fn lower_metal_draw_scene(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();

    // Every one of the three collections is addressed as a flat payload at
    // `block + COLLECTION_HEADER_SIZE`, which only holds while they keep the
    // entry-free representation. The same check `canvas::blitSurface` makes, for the
    // same reason: if one of these regained a lookup table the renderer would read
    // entry records as pixels or as geometry, and the result would look like a
    // rasteriser bug rather than a layout change.
    for element in [
        ParameterType::Byte,
        ParameterType::Float,
        ParameterType::Integer,
    ] {
        let stride = list_entry_stride(&element);
        if stride != 0 {
            return Err(format!(
                "'{symbol}' assumes `List OF {}` is entry-free, but its entry stride is \
                 {stride} — the payload is no longer contiguous",
                element.name(),
            ));
        }
    }

    let mut located = Vec::new();
    for (index, what) in [
        "the surface argument",
        "the width argument",
        "the height argument",
        "the geometry argument",
        "the offsets argument",
        "the glyph metadata argument",
        "the glyph coverage argument",
        // plan-116-H Phase 3: the draw list, the same eighth parameter the Vulkan twin
        // takes. Metal reached this letter still locating seven while `scene_params()`
        // -- shared by both functions -- already declared eight, so the list was
        // accepted at the call and then dropped on the floor.
        "the draw list argument",
    ]
    .into_iter()
    .enumerate()
    {
        located.push(
            args.get(index)
                .ok_or_else(|| format!("'{symbol}' expects {what}"))?
                .location
                .clone(),
        );
    }

    // **Every value is computed into a temporary before ANY argument register is
    // written.** The incoming arguments are themselves in the MFB argument bank, so
    // staging in place makes each write a potential clobber of a later read — and it
    // does not stay theoretical: with the glyph cache added, `located[5]` arrives in the
    // register `mfb_arg(5)` names, so loading the offset count straight into it
    // destroyed the glyph metadata pointer, and the next instruction derived the
    // coverage pointer from the wreckage. The graphics thread then segfaulted
    // dereferencing 0x29. Two passes cost a few moves the allocator mostly coalesces
    // away, and cannot be wrong.
    //
    // The bank is eight wide and this seam fills it exactly. plan-116-H needed a ninth
    // value — the draw list — so one slot was bought back rather than spending a stack
    // argument: `offsets` now travels as its **collection pointer** instead of as a
    // (payload, count) pair, and `emit_metal_draw` derives both from it, which is what
    // the Vulkan emitter already does with every list it is handed.
    //
    // That keeps the property the old comment was defending. The count still comes off
    // the collection header rather than from a caller-supplied argument, so a caller
    // cannot pass a count that disagrees with the list it also passed — it is now
    // simply read one level further in.
    //
    // Slots and what they carry, after the change:
    //
    //   0 surface payload   1 width            2 height          3 geometry payload
    //   4 offsets POINTER   5 glyph metadata   6 glyph coverage  7 draws POINTER
    //
    // Slots 4 and 7 are the odd ones out. Passing a payload where the emitter expects a
    // pointer reads the collection header as data -- a plausible wrong picture rather
    // than a fault -- and the only thing that catches it is a render compared against
    // the software oracle: `every_group_case_matches_the_software_oracle` in
    // `tests/rt_canvas_metal.rs`, which needs the draw list to be read correctly before
    // any of its seven cases can land in the right place.
    let mut staged = Vec::new();
    for (slot, source) in [(0usize, 0usize), (3, 3), (5, 5), (6, 6)] {
        let payload = builder.temporary_vreg();
        builder.emit(abi::add_immediate(
            &payload,
            &located[source],
            COLLECTION_HEADER_SIZE,
        ));
        staged.push((slot, payload));
    }
    // The two that travel whole. Still staged through temporaries for the reason above:
    // `located[4]` and `located[7]` arrive in argument registers this loop overwrites.
    for (slot, source) in [(4usize, 4usize), (7, 7)] {
        let whole = builder.temporary_vreg();
        builder.emit(abi::move_register(&whole, &located[source]));
        staged.push((slot, whole));
    }
    let width = builder.temporary_vreg();
    let height = builder.temporary_vreg();
    builder.emit(abi::move_register(&width, &located[1]));
    builder.emit(abi::move_register(&height, &located[2]));

    for (slot, payload) in staged {
        builder.emit(abi::move_register(abi::mfb_arg(slot), &payload));
    }
    builder.emit(abi::move_register(abi::mfb_arg(1), &width));
    builder.emit(abi::move_register(abi::mfb_arg(2), &height));

    if let Some(result) =
        ctx.platform
            .emit_metal_draw(&symbol, &mut builder.instructions, &mut builder.relocations)
    {
        result?;
    }

    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

/// `canvas::vulkanDrawScene(surface, width, height, geometry, offsets) AS Nothing`.
///
/// The Vulkan twin of `metalDrawScene`, and the same contract: it writes **through**
/// the surface argument rather than returning a new collection, because the buffer
/// comes straight from `canvas::newSurface` inside `__canvas_renderVulkan` and is
/// aliased by nothing.
///
/// Unlike the Metal one this needs no platform seam: Vulkan is plain C reached
/// through `dlopen`, so the whole emitter is target-neutral and lives in
/// `runtime/canvas/vulkan.rs`. On a target with no Vulkan path it emits nothing and
/// the call is a no-op — unreachable anyway, since the renderer branch gates on
/// `canvas::vulkanReady`.
pub(crate) fn lower_vulkan_draw_scene(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let mut located = Vec::new();
    for (index, what) in [
        "the surface argument",
        "the width argument",
        "the height argument",
        "the geometry argument",
        "the offsets argument",
        "the glyph metadata argument",
        "the glyph coverage argument",
        "the draw list argument",
    ]
    .into_iter()
    .enumerate()
    {
        located.push(
            args.get(index)
                .ok_or_else(|| format!("'{symbol}' expects {what}"))?
                .location
                .clone(),
        );
    }
    emit_vulkan_draw_scene(
        builder,
        ctx.platform,
        ctx.platform_imports,
        &located[0],
        &located[1],
        &located[2],
        &located[3],
        &located[4],
        &located[5],
        &located[6],
        &located[7],
    )?;
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());
    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: symbol,
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "metalDrawScene",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: scene_params(),
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_metal_draw_scene),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "vulkanDrawScene",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: scene_params(),
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_vulkan_draw_scene),
        }],
    });
}

/// The parameter list both GPU draw seams take: the surface to write, its
/// dimensions, the geometry cache, the per-item offsets in draw order, and the glyph
/// coverage cache a `Text` item's run indexes into.
///
/// The glyph cache arrives as two lists rather than being reachable from the geometry:
/// a run stores cache *indices*, and the bitmaps live in globals the emitters cannot
/// name. Passing them is what lets a backend draw text at all — without them a glyph
/// run is three numbers per character and no pixels.
fn scene_params() -> Vec<Parameter> {
    vec![
        Parameter {
            name: "surface",
            desc: "",
            aliases: &[],
            ty: ParameterType::list_of(ParameterType::Byte),
            default: DefaultValue::None,
        },
        Parameter {
            name: "width",
            desc: "",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        },
        Parameter {
            name: "height",
            desc: "",
            aliases: &[],
            ty: ParameterType::Integer,
            default: DefaultValue::None,
        },
        Parameter {
            name: "geometry",
            desc: "",
            aliases: &[],
            ty: ParameterType::list_of(ParameterType::Float),
            default: DefaultValue::None,
        },
        Parameter {
            name: "offsets",
            desc: "",
            aliases: &[],
            ty: ParameterType::list_of(ParameterType::Integer),
            default: DefaultValue::None,
        },
        Parameter {
            name: "glyphMeta",
            desc: "",
            aliases: &[],
            ty: ParameterType::list_of(ParameterType::Integer),
            default: DefaultValue::None,
        },
        Parameter {
            name: "glyphCoverage",
            desc: "",
            aliases: &[],
            ty: ParameterType::list_of(ParameterType::Byte),
            default: DefaultValue::None,
        },
        // plan-116-H: the per-draw list, four integers per entry —
        // `(itemBase, itemCount, dx, dy)` with the offsets in 16.16. `offsets` above is
        // now the flat BLOCK list a base indexes into, in which a shared group appears
        // once; this says who draws which slice of it, and where.
        //
        // The eighth parameter, which is the one MFBASIC's convention puts in `rbp`
        // (bug-296). That is safe here and the reason is worth stating, because the
        // natural reading of "up to 8" is that eight is a ceiling to stay under: it is
        // not, arguments past the eighth simply go on the stack. What actually matters
        // is that every point where FOREIGN code calls into MFB code saves `rbp`, and
        // this letter adds no such point — every call here is MFB→MFB (plan-116-G G31,
        // which recorded the wrong version of this rule first and then corrected it).
        Parameter {
            name: "draws",
            desc: "",
            aliases: &[],
            ty: ParameterType::list_of(ParameterType::Integer),
            default: DefaultValue::None,
        },
    ]
}
