//! `canvas::destroyFont` — release a font early.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

const INTRO: &str = r#"Release a font before it leaves scope."#;

const DESC: &str = r#"`destroyFont` closes the font, exactly as `fs::close` closes a file: the handle is
released now rather than when the binding goes out of scope. Letting a `canvas::Font` leave
scope does the same thing, so this is for the case where a font is large and the
scope is long — a font file is measured in hundreds of kilobytes, so this is a more
useful call than its image twin.

**It is safe at any time, including while a presented scene still draws text in the
font.** A scene carries the id, not the font, so it cannot dangle; text naming a
released font measures and draws as empty rather than faulting.

Closing twice is the defined no-op, and using a closed font afterwards raises the
universal `ErrResourceClosed` — the same contract every resource has.

Unlike the rest of `canvas`, `destroyFont` does **not** require `app::Mode.Canvas`: a
program leaving canvas mode must still be able to release what it allocated, and
closing a handle touches no surface."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas

SUB main()
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("DejaVuSans.ttf")
  canvas::destroyFont(face)
END SUB
```"#;

/// `canvas::destroyFont(font)` — set the closed bit.
///
/// Unconditional, like `destroyImage`: double-close must be a no-op rather than an
/// error (the universal resource contract), so storing `1` over `1` is the whole
/// operation and testing first would only add a branch to reach the same state. The
/// font bytes are arena-owned and reclaimed with the arena; there is no OS handle to
/// give back.
pub(crate) fn lower_destroy_font(
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

    // Unpublish before closing: a renderer that looked the handle up between the two
    // would find a block whose resource is already closed, and the whole point of the
    // handle indirection is that a scene never has to care.
    let handle = builder.temporary_vreg();
    builder.emit(abi::load_u64(&handle, &record, RESOURCE_OFFSET_HANDLE));
    super::gen_font_table::emit_unregister_font(builder, &Operand::from(handle.to_string()));

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
        text: "canvas.destroyFont".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "destroyFont",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "font",
                desc: "The font to release. Safe to call twice, and safe while a \
                       presented scene still draws text in it.",
                aliases: &[],
                ty: ParameterType::named(super::FONT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_destroy_font),
        }],
    });
}
