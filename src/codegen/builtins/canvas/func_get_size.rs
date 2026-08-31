//! `canvas::getSize` — an image's pixel dimensions, or the surface's.
//!
//! Two overloads on one name, distinguished by arity. They answer the same
//! question — "how big is the thing I am drawing on or with?" — and a program
//! laying a `Picture` out proportionally needs both in the same expression, so
//! splitting them across two names would put an arbitrary distinction in the
//! caller's way.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_image::{emit_closed_guard, IMAGE_HEIGHT, IMAGE_WIDTH};

const INTRO: &str = r#"The pixel dimensions of an image, or of the canvas surface."#;

const DESC: &str = r#"`getSize(image)` returns the `width` and `height` an image was created with, so a
program can lay a `Picture` out proportionally without tracking the numbers itself.

`getSize()` with no argument returns the **canvas surface** size instead — the
drawing area a scene is presented into, in the same pixel coordinates every
`DrawItem` uses. That is what a program centres on.

It reads the runtime's own record — no backend round trip — so it costs a load.

Raises `ErrResourceClosed` if the image has been destroyed."#;

const EX: &str = r#"Draw an image at its natural size:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET px AS List OF Byte = [toByte(255), toByte(0), toByte(0), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET size AS Size = canvas::getSize(img)
  LET tile AS DrawItem = Picture[x := 0.0, y := 0.0, w := toFloat(size.width), h := toFloat(size.height), image := canvas::imageRef(img), paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([tile])
END SUB
```"#;

/// `canvas::getSize(image) AS Size` — a two-field record from `width`/`height`.
pub(crate) fn lower_get_size(
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

    let closed = builder.label("canvas_get_size_closed");
    let alloc_ok = builder.label("canvas_get_size_alloc_ok");
    let done = builder.label("canvas_get_size_done");

    let record_slot = builder.allocate_stack_object("canvas_get_size_rec", 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    emit_closed_guard(builder, &record, &closed);

    // `Size` is two `Integer`s.
    builder.emit(abi::move_immediate(abi::c_arg(0), "Integer", "16"));
    builder.emit(abi::move_immediate(abi::c_arg(1), "Integer", "8"));
    builder.emit_arena_alloc_call();
    builder.emit(abi::branch_eq(&alloc_ok));
    builder.raise_error_bare("ErrOutOfMemory")?;
    builder.emit(abi::branch(&done));
    builder.emit(abi::label(&alloc_ok));

    let size = builder.temporary_vreg();
    builder.emit(abi::move_register(&size, abi::mfb_return(1)));
    let source = builder.temporary_vreg();
    let value = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&value, &source, IMAGE_WIDTH));
    builder.emit(abi::store_u64(&value, &size, 0));
    builder.emit(abi::load_u64(&value, &source, IMAGE_HEIGHT));
    builder.emit(abi::store_u64(&value, &size, 8));

    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &size));
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
        text: "canvas.getSize".to_string(),
    })
}

/// `canvas::getSize() AS Size` — the canvas surface's dimensions.
///
/// Delegates to `__canvas_surfaceSize` rather than repeating the numbers, so the
/// renderer and the program cannot disagree about how big the surface is — and so
/// plan-98-D's live resize has one definition to replace instead of two.
#[rustfmt::skip]
const SURFACE_BODY: &str =
r#"FUNC __canvas_getSurfaceSize() AS Size
  RETURN __canvas_surfaceSize()
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getSize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![
            Implementation {
                params: vec![Parameter {
                    name: "image",
                    desc: "The image to measure.",
                    aliases: &[],
                    ty: ParameterType::named(super::IMAGE_TYPE_ID),
                    default: DefaultValue::None,
                }],
                return_type: ParameterType::named("Size"),
                errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
                body: Body::abi_function(lower_get_size),
            },
            Implementation {
                params: vec![],
                return_type: ParameterType::named("Size"),
                errors: vec![],
                body: Body::mfb(SURFACE_BODY, "__canvas_getSurfaceSize"),
            },
        ],
    });
}
