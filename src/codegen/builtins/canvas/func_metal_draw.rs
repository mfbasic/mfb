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

    // The offset count comes off the collection header rather than from a sixth
    // MFBASIC argument: `len(offsets)` at the call site and `count` in the block are
    // the same number, and reading it here means the caller cannot pass a count that
    // disagrees with the list it also passed.
    builder.emit(abi::load_u64(
        abi::mfb_arg(5),
        &located[4],
        COLLECTION_OFFSET_COUNT,
    ));
    for (slot, source) in [(0usize, 0usize), (3, 3), (4, 4)] {
        builder.emit(abi::add_immediate(
            abi::mfb_arg(slot),
            &located[source],
            COLLECTION_HEADER_SIZE,
        ));
    }
    builder.emit(abi::move_register(abi::mfb_arg(1), &located[1]));
    builder.emit(abi::move_register(abi::mfb_arg(2), &located[2]));

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

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "metalDrawScene",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
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
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_metal_draw_scene),
        }],
    });
}
