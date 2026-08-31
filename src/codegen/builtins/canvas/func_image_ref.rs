//! `canvas::imageRef` — the value handle a scene carries in place of an `Image`.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_image::emit_closed_guard;

const INTRO: &str = r#"Take a plain `canvas::ImageRef` handle naming an `Image`."#;

const DESC: &str = r#"`imageRef` produces the value a `canvas::Picture` item carries: the id the backend knows
the image by, and nothing else.

This exists because **a scene must not hold a resource.** A `canvas::DrawItem` is a record,
and a record field cannot hold a `RES` value; more importantly, a published scene
outlives the call that installed it and is read by the renderer at arbitrary times,
so a scene holding resources would have to keep them open — which would make
`canvas::destroyImage` a lie. Holding only the id means an installed scene never
keeps an image open.

An id naming a destroyed image is harmless: it is just an integer, and the
runtime stops drawing it once the image is gone.

Raises `ErrResourceClosed` if the image has already been destroyed — taking a
handle to something that no longer exists is a program error, unlike drawing a
handle whose image was destroyed afterwards, which is fine."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET px AS List OF Byte = [toByte(0), toByte(255), toByte(0), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET tile AS DrawItem = Picture[x := 0.0, y := 0.0, w := 32.0, h := 32.0, image := canvas::imageRef(img), paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([tile])
END SUB
```"#;

/// `canvas::imageRef(image) AS ImageRef` — read `handle@8` into a one-field record.
///
/// The result is a *value*, so it is built in the arena like any other record rather
/// than aliasing the resource: an `ImageRef` copied into a scene must survive the
/// resource it names.
pub(crate) fn lower_image_ref(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let record = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the image argument"))?
        .location
        .clone();

    let closed = builder.label("canvas_image_ref_closed");
    let alloc_ok = builder.label("canvas_image_ref_alloc_ok");
    let done = builder.label("canvas_image_ref_done");

    let record_slot = builder.allocate_stack_object("canvas_image_ref_rec", 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    emit_closed_guard(builder, &record, &closed);

    // One `Integer` field, so an 8-byte block.
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "8"));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&alloc_ok));

    let handle_record = builder.temporary_vreg();
    builder.emit(abi::move_register(&handle_record, abi::mfb_return(1)));
    let source = builder.temporary_vreg();
    let handle = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&handle, &source, RESOURCE_OFFSET_HANDLE));
    builder.emit(abi::store_u64(&handle, &handle_record, 0));

    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &handle_record));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&closed));
    builder.raise_error_bare("ErrResourceClosed")?;

    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.imageRef".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "imageRef",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "image",
                desc: "The image to name.",
                aliases: &[],
                ty: ParameterType::named(super::IMAGE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("ImageRef"),
            errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
            body: Body::abi_function(lower_image_ref),
        }],
    });
}
