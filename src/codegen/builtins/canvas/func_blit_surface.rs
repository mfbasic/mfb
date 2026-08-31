//! `canvas::blitSurface` — hand a finished frame to the platform's canvas surface.
//!
//! Internal-only: a program presents a *scene*, and where the resulting pixels go is
//! the runtime's business. This is the call `__canvas_presentSurface` makes after the
//! rasteriser has filled the buffer.
//!
//! The body is entirely a [`CodegenPlatform::emit_canvas_blit`] seam, because there
//! is no portable part. Each backend owns a different surface object (a layer-backed
//! `NSView`, a `GtkDrawingArea`, an `HWND`), and each may only be touched from its UI
//! thread while this call arrives on the worker. What they share is only the
//! signature.
//!
//! A target with no canvas surface returns `None` from the seam and this lowers to a
//! plain return. That is deliberate rather than an error: rendering still happened,
//! and the `MFB_CANVAS_DUMP` path — which is what the golden harness reads — does not
//! need a window at all.

// --- codegen tier imports (migration) ---
use crate::codegen::collection::layout::byte_list_entry_stride;
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

/// `canvas::blitSurface(buffer AS List OF Byte, width AS Integer, height AS Integer)`.
pub(crate) fn lower_blit_surface(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();

    // A `List OF Byte` uses the entry-free representation, so its payload is already a
    // contiguous RGBA8 block at `collection + COLLECTION_HEADER_SIZE` and the backend
    // can hand that pointer straight to the platform's image API. Flattening it into a
    // scratch buffer first — the shape `emit_read_byte_list` gives — would allocate and
    // copy 2.3 MB per frame for nothing.
    //
    // The check is what keeps that from silently becoming wrong: if `List OF Byte` ever
    // regains a lookup table, the payload stops being contiguous and this pointer would
    // address entry records as if they were pixels. Failing the build is the only
    // acceptable outcome, because the rendered garbage would read as a rasteriser bug.
    let stride = byte_list_entry_stride();
    if stride != 0 {
        return Err(format!(
            "'{symbol}' assumes `List OF Byte` is entry-free, but its entry stride is \
             {stride} — the frame payload is no longer contiguous",
        ));
    }

    let buffer = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the frame buffer argument"))?
        .location
        .clone();
    let width = args
        .get(1)
        .ok_or_else(|| format!("'{symbol}' expects the width argument"))?
        .location
        .clone();
    let height = args
        .get(2)
        .ok_or_else(|| format!("'{symbol}' expects the height argument"))?
        .location
        .clone();

    // Stage the platform contract: pixels, width, height.
    builder.emit(abi::add_immediate(
        abi::mfb_arg(0),
        &buffer,
        COLLECTION_HEADER_SIZE,
    ));
    builder.emit(abi::move_register(abi::mfb_arg(1), &width));
    builder.emit(abi::move_register(abi::mfb_arg(2), &height));

    if let Some(result) =
        ctx.platform
            .emit_canvas_blit(&symbol, &mut builder.instructions, &mut builder.relocations)
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
        name: "blitSurface",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "buffer",
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
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_blit_surface),
        }],
    });
}
