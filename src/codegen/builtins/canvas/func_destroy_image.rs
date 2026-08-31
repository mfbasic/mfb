//! `canvas::destroyImage` — release an image early.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Release an image before it leaves scope."#;

const DESC: &str = r#"`destroyImage` closes the image, exactly as `fs::close` closes a file: the handle
is closed now rather than when the binding goes out of scope. Letting an `Image`
leave scope does the same thing, so this is for the case where an image is large
and the scope is long.

**It is safe at any time, including while a presented scene still names the
image.** A scene carries the id, not the image, so destroying the image leaves
the scene intact — the runtime simply stops drawing that item.

Closing twice is the defined no-op, and using a closed image afterwards raises the
universal `ErrResourceClosed` — the same contract every resource has.

Unlike the rest of `canvas`, `destroyImage` does **not** require `Mode.Canvas`: a
program leaving canvas mode must still be able to close the images it made, and
closing a handle touches no surface."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  LET px AS List OF Byte = [toByte(255), toByte(255), toByte(255), toByte(255)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  canvas::destroyImage(img)
END SUB
```"#;

/// `canvas::destroyImage(image)` — set the closed bit.
///
/// Double-close must be a no-op rather than an error (the universal resource
/// contract), so this is an unconditional store of the closed flag: storing `1` over
/// `1` changes nothing, and testing first would only add a branch to reach the same
/// state. The OS-side free is deliberately not here — it is gated on
/// `closed AND lastUsedFrame < lastCompletedFrame` and belongs to the backend
/// (plan-98-D), because only the backend knows when the GPU is done reading.
pub(crate) fn lower_destroy_image(
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

    let flag = builder.temporary_vreg();
    builder.emit(abi::move_immediate(&flag, "Integer", "1"));
    builder.emit(abi::store_u64(&flag, &record, RESOURCE_OFFSET_CLOSED));
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
        text: "canvas.destroyImage".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "destroyImage",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "image",
                desc: "The image to destroy. Safe to call twice, and safe while a \
                       presented scene still draws it.",
                aliases: &[],
                ty: ParameterType::named(super::IMAGE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_destroy_image),
        }],
    });
}
