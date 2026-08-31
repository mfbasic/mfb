//! `canvas::fontRef` — the value handle a scene carries in place of a `Font`.

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

const INTRO: &str = r#"Take a plain `FontRef` handle naming a `Font`."#;

const DESC: &str = r#"`fontRef` produces the value a `Text` item carries: the id the backend knows the
font by, and nothing else.

This exists for the same reason `canvas::imageRef` does — **a scene must not hold a
resource.** A `DrawItem` is a record and a record field cannot hold a `RES` value;
more importantly a published scene outlives the call that installed it and is read
by the renderer at arbitrary times, so a scene holding resources would have to keep
them alive, which would make `canvas::destroyFont` a lie. Holding only the id means
an installed scene has no opinion about any font's lifetime at all.

A handle naming a destroyed font is not dangling: it is an integer, and text
carrying it measures and draws as empty once the font is gone.

Raises `ErrResourceClosed` if the font has already been destroyed — taking a handle
to something that no longer exists is a program error, unlike drawing a handle whose
font was destroyed afterwards, which is fine."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("DejaVuSans.ttf")
  LET label AS DrawItem = Text[x := 10.0, y := 40.0, text := "hello", font := canvas::fontRef(face), size := 24.0, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::present([label])
END SUB
```"#;

/// `canvas::fontRef(font) AS FontRef` — read `handle@8` into a one-field record.
///
/// A copy of `lower_image_ref` in shape, deliberately: the two handles differ only in
/// which resource they name, and the shared piece worth sharing — the closed guard —
/// already is (`gen_image::emit_closed_guard`). Folding the rest into one generic
/// emitter would trade four obvious instructions for a parameter that means "which
/// error message", which is how a seam stops being readable.
///
/// The result is a *value*, so it is built in the arena like any other record rather
/// than aliasing the resource: a `FontRef` copied into a scene must survive the
/// resource it names.
pub(crate) fn lower_font_ref(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    _ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let record = args
        .first()
        .ok_or_else(|| format!("'{symbol}' expects the font argument"))?
        .location
        .clone();

    let closed = builder.label("canvas_font_ref_closed");
    let alloc_ok = builder.label("canvas_font_ref_alloc_ok");
    let done = builder.label("canvas_font_ref_done");

    let record_slot = builder.allocate_stack_object("canvas_font_ref_rec", 8);
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
        text: "canvas.fontRef".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fontRef",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "font",
                desc: "The font to name.",
                aliases: &[],
                ty: ParameterType::named(super::FONT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::named("FontRef"),
            errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
            body: Body::abi_function(lower_font_ref),
        }],
    });
}
