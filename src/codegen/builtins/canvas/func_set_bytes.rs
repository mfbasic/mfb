//! `canvas::setBytes` — replace an image's pixels.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_image::{emit_closed_guard, IMAGE_DIRTY, IMAGE_PIXELS};

const INTRO: &str = r#"Replace an image's pixels."#;

const DESC: &str = r#"`setBytes` replaces the image's contents with `pixels` — `width * height * 4` bytes
of RGBA8 in row order, top row first. Any other length raises `ErrBadPixelCount`;
an image cannot be resized, only re-filled.

**This does not go through `canvas::present`.** Changing an image's pixels does not
change the scene: the same items are in the same places, and only the content
behind one of their ids is different. So there is no scene to re-install, and the
new pixels appear on the next rendered frame. That is the whole reason images are
identified by handle rather than embedded in items — a video frame, a plot, or a
progress bar can update without rebuilding the scene at all.

The pixels are copied, so you may reuse or discard the list immediately.

Raises `ErrResourceClosed` if the image has been destroyed."#;

const EX: &str = r#"Update a tile's contents without re-presenting:

```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  LET black AS List OF Byte = [toByte(0), toByte(0), toByte(0), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(1, 1, black)
  LET tile AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 16.0, h := 16.0, image := canvas::imageRef(img), paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([tile])

  ' The scene is unchanged; only the pixels behind the id are.
  LET white AS List OF Byte = [toByte(255), toByte(255), toByte(255), toByte(255)]
  canvas::setBytes(img, white)
END SUB
```"#;

/// `canvas::setBytes(image, pixels)`.
///
/// The length check compares against the **existing shadow's** byte count rather
/// than recomputing `width * height * 4`. Both are the same number, but the shadow's
/// count is the one the image actually has, so the check cannot drift from reality
/// if the dimensions and the shadow ever disagree — and it needs no multiply, hence
/// no overflow case to get right.
pub(crate) fn lower_set_bytes(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    if args.len() < 2 {
        return Err(format!("'{symbol}' expects the image and the pixel list"));
    }
    let record = args[0].location.clone();
    let pixels_in = args[1].location.clone();

    let closed = builder.label("canvas_set_bytes_closed");
    let bad_count = builder.label("canvas_set_bytes_bad_count");
    let done = builder.label("canvas_set_bytes_done");

    let record_slot = builder.allocate_stack_object("canvas_set_bytes_rec", 8);
    let pixels_slot = builder.allocate_stack_object("canvas_set_bytes_px", 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    builder.emit(abi::store_u64(
        &pixels_in,
        abi::stack_pointer(),
        pixels_slot,
    ));
    emit_closed_guard(builder, &record, &closed);

    // Compare the incoming count against the shadow's.
    let source = builder.temporary_vreg();
    let shadow = builder.temporary_vreg();
    let expected = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&shadow, &source, IMAGE_PIXELS));
    builder.emit(abi::load_u64(&expected, &shadow, COLLECTION_OFFSET_COUNT));
    let pixels = builder.temporary_vreg();
    let actual = builder.temporary_vreg();
    builder.emit(abi::load_u64(&pixels, abi::stack_pointer(), pixels_slot));
    builder.emit(abi::load_u64(&actual, &pixels, COLLECTION_OFFSET_COUNT));
    builder.emit(abi::compare_registers(&actual, &expected));
    builder.emit(abi::branch_ne(&bad_count));

    // Copy into a fresh shadow and swap it in. Replacing the block rather than
    // overwriting in place keeps this correct when the old shadow is still being
    // read — the backend may be uploading from it on another thread.
    let pixels_again = builder.temporary_vreg();
    builder.emit(abi::load_u64(
        &pixels_again,
        abi::stack_pointer(),
        pixels_slot,
    ));
    let fresh =
        builder.copy_flat_block(&ParameterType::list_of(ParameterType::Byte), &pixels_again)?;
    let target = builder.temporary_vreg();
    builder.emit(abi::load_u64(&target, abi::stack_pointer(), record_slot));
    builder.emit(abi::store_u64(&fresh, &target, IMAGE_PIXELS));
    // Mark dirty AFTER the pointer swap, so a reader that sees the dirty flag is
    // guaranteed to see the new pixels rather than the old ones.
    let one = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&one, "Integer", "1"));
    builder.emit(abi::store_u64(&one, &target, IMAGE_DIRTY));

    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&bad_count));
    builder.raise_error_bare("ErrBadPixelCount")?;
    builder.emit(abi::branch(&done));

    builder.emit(abi::label(&closed));
    builder.raise_error_bare("ErrResourceClosed")?;

    builder.emit(abi::label(&done));
    builder.emit(abi::return_());

    Ok(ValueResult {
        origin: None,
        type_: ParameterType::Nothing,
        location: Operand::from("void"),
        text: "canvas.setBytes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "image",
                    desc: "The image to refill.",
                    aliases: &[],
                    ty: ParameterType::named(super::IMAGE_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "pixels",
                    desc: "RGBA8 bytes in row order — exactly as many as the image \
                           already holds. An image cannot be resized.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec!["ErrBadPixelCount", "ErrResourceClosed", "ErrOutOfMemory"],
            body: Body::abi_function(lower_set_bytes),
        }],
    });
}
