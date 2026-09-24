//! `canvas::metalPresentScene` — render one frame on the GPU and present it straight
//! to the window (bug-686 Phase 4).
//!
//! Internal-only, and the sibling of `canvas::metalDrawScene`: the same geometry, the
//! same render, but no surface. `metalDrawScene` reads the finished frame back into a
//! CPU `List OF Byte` so it can leave through `canvas::blitSurface` as a `CGImage`; at
//! a full-screen surface that readback, the BGRA->RGBA swizzle, the surface allocation
//! and the image blit are the whole frame budget. This one blits the offscreen target
//! into the window `CAMetalLayer`'s next drawable on the GPU and presents it.
//!
//! It answers whether it did. FALSE is not an error: it means there is nowhere to
//! present — a headless run builds no window layer, so every headless frame answers
//! FALSE before touching Metal — or no drawable was available this frame. The caller
//! (`__canvas_renderMetal`) then renders the frame down the readback path, which is why
//! the oracle tests, all headless, still compare exactly the pixels they always did.
//!
//! It reaches the renderer through the SAME platform seam as `metalDrawScene`
//! (`emit_metal_draw`), with the surface slot staged as NULL — the renderer's
//! "present directly" signal (`emit_metal_draw` in the macOS app module). A second
//! seam would be a second contract for one function; a real surface payload is
//! `block + COLLECTION_HEADER_SIZE` and is never NULL, so the two cannot be confused.
//! A target with no Metal renderer has no seam and answers FALSE.

use crate::codegen::collection::layout::list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `canvas::metalPresentScene(width, height, geometry, offsets, glyphMeta,
/// glyphCoverage, draws) AS Boolean`.
pub(crate) fn lower_metal_present_scene(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();

    // The geometry, the glyph coverage and the two integer lists are addressed as flat
    // payloads at `block + COLLECTION_HEADER_SIZE` — the check `metalDrawScene` makes,
    // for the same reason: a list that regained a lookup table would have its entry
    // records read as geometry.
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

    // The renderer's argument bank, exactly as `metalDrawScene` fills it except for
    // slot 0 — the surface payload there, NULL here:
    //
    //   0 NULL              1 width            2 height          3 geometry payload
    //   4 offsets POINTER   5 glyph metadata   6 glyph coverage  7 draws POINTER
    //
    // Every value is computed into a temporary before ANY argument register is
    // written, for the reason `lower_metal_draw_scene` documents: the incoming
    // arguments are themselves in the MFB argument bank, and this bank is shifted by
    // one against it, so staging in place would overwrite a later read.
    let mut staged = Vec::new();
    for (slot, source) in [(3usize, 2usize), (5, 4), (6, 5)] {
        let payload = builder.temporary_vreg();
        builder.emit(abi::add_immediate(
            &payload,
            &located[source],
            COLLECTION_HEADER_SIZE,
        ));
        staged.push((slot, payload));
    }
    for (slot, source) in [(1usize, 0usize), (2, 1), (4, 3), (7, 6)] {
        let whole = builder.temporary_vreg();
        builder.emit(abi::move_register(&whole, &located[source]));
        staged.push((slot, whole));
    }
    for (slot, value) in staged {
        builder.emit(abi::move_register(abi::mfb_arg(slot), &value));
    }
    builder.emit(abi::move_immediate(abi::mfb_arg(0), "Integer", "0"));

    match ctx
        .platform
        .emit_metal_draw(&symbol, &mut builder.instructions, &mut builder.relocations)
    {
        Some(result) => {
            result?;
            // The renderer answers 1 (presented) or 0 (not drawn) in the C return
            // register; anything non-zero is TRUE.
            builder.emit(abi::move_register(RESULT_VALUE_REGISTER, abi::c_return(0)));
        }
        None => builder.emit(abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0")),
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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    let list = |name: &'static str, element: ParameterType| Parameter {
        name,
        desc: "",
        aliases: &[],
        ty: ParameterType::list_of(element),
        default: DefaultValue::None,
    };
    let integer = |name: &'static str| Parameter {
        name,
        desc: "",
        aliases: &[],
        ty: ParameterType::Integer,
        default: DefaultValue::None,
    };
    pkg.add_function(RegistryFunction {
        name: "metalPresentScene",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            // `metalDrawScene`'s parameters without the surface — see its
            // `scene_params` for what each carries.
            params: vec![
                integer("width"),
                integer("height"),
                list("geometry", ParameterType::Float),
                list("offsets", ParameterType::Integer),
                list("glyphMeta", ParameterType::Integer),
                list("glyphCoverage", ParameterType::Byte),
                list("draws", ParameterType::Integer),
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_metal_present_scene),
        }],
    });
}
