//! `canvas::getBytes` — an image's current pixels.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_image::{emit_closed_guard, IMAGE_PIXELS};

const INTRO: &str = r#"The image's current RGBA8 pixels."#;

const DESC: &str = r#"`getBytes` returns the image's pixels — `width * height * 4` bytes of RGBA8 in row
order, top row first.

It is **cheap**: the runtime keeps its own copy of every image's pixels as the
source of truth the backend draws from, so this reads that copy rather than
asking the GPU to read anything back. A readback would be a full pipeline stall;
this is a memory copy.

The result is an ordinary `List OF Byte` value, so mutating it does not touch the
image — write pixels back with `canvas::setBytes`.

Raises `ErrResourceClosed` if the image has been destroyed."#;

const EX: &str = r#"Read an image's pixels back and report its first red channel:

```
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT io

SUB main()
  app::setMode(app::Mode.Canvas)
  LET px AS List OF Byte = [toByte(10), toByte(20), toByte(30), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET current AS List OF Byte = canvas::getBytes(img)
  io::print(toString(collections::getOr(current, 0, toByte(0))))
END SUB
```"#;

/// `canvas::getBytes(image) AS List OF Byte`.
///
/// The shadow is **copied**, not returned by reference. MFBASIC collections are
/// values, so handing back the runtime's own block would let a caller mutate the
/// image's contents behind its back — and worse, would alias storage the runtime
/// later replaces on `setBytes`.
pub(crate) fn lower_get_bytes(
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

    let closed = builder.label("canvas_get_bytes_closed");
    let done = builder.label("canvas_get_bytes_done");

    let record_slot = builder.allocate_stack_object("canvas_get_bytes_rec", 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    emit_closed_guard(builder, &record, &closed);

    let source = builder.temporary_vreg();
    let shadow = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&shadow, &source, IMAGE_PIXELS));
    let copy = builder.copy_flat_block(&ParameterType::list_of(ParameterType::Byte), &shadow)?;

    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &copy));
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
        text: "canvas.getBytes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "getBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "image",
                desc: "The image to read.",
                aliases: &[],
                ty: ParameterType::named(super::IMAGE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
            body: Body::abi_function(lower_get_bytes),
        }],
    });
}
