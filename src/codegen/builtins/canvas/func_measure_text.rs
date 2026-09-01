//! `canvas::measureText` and the internal `canvas::fontBytes` it reads through.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::target::shared::abi;
use crate::types::ParameterType;

use super::gen_font::FONT_BYTES;
use super::gen_image::emit_closed_guard;

const INTRO: &str = r#"Measure a string in a font without drawing it."#;

const DESC: &str = r#"`measureText` returns the `TextMetrics` a `Text` item would occupy: the advance
`width` of the string, and the font's `ascent`, `descent` and `lineGap` scaled to
`size`. `height` is the full line height, `ascent + descent + lineGap`, so stacking
lines is repeated addition of one number.

All five numbers are in pixels at the given `size`, scaled from the font's design
grid by `size / unitsPerEm`. `descent` is a **positive** distance below the baseline.
The font file stores it negative — `hhea.descender` measures downward — and the sign
is flipped here so no caller has to remember which convention it is holding.

`width` is the sum of the glyphs' advance widths. This build does no kerning,
ligatures or complex shaping (see `canvas::loadFont`), so the width is exact for what
it will actually draw — measuring and drawing use the same glyph walk, which is the
property that matters more than absolute typographic fidelity.

A font with no `head` table, or a `FontRef` naming a released font, measures as all
zeroes rather than failing: a program that lays out text before its font is ready
should get an empty box, not an error in the middle of a frame."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas
IMPORT io

SUB main()
  app::setMode(Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("DejaVuSans.ttf")
  LET m AS TextMetrics = canvas::measureText(face, 24.0, "hello")
  io::print("width " & toString(m.width) & " height " & toString(m.height))
END SUB
```"#;

/// `canvas::measureText(font, size, text)` — sum the advances, scale the metrics.
///
/// MFBASIC, over the table readers in `helper_font.rs`. The one thing it cannot do
/// itself is reach the resource's bytes, which is what `canvas::fontBytes` is for.
#[rustfmt::skip]
const MEASURE_TEXT: &str =
r#"FUNC __canvas_measureText(font AS canvas::Font, size AS Float, text AS String) AS TextMetrics
  LET b AS List OF Byte = canvas::fontBytes(font)
  LET upem AS Integer = __canvas_fontUnitsPerEm(b)
  IF upem <= 0 THEN
    RETURN TextMetrics[width := 0.0, height := 0.0, ascent := 0.0, descent := 0.0, lineGap := 0.0]
  END IF
  LET scale AS Float = size / toFloat(upem)
  MUT advance AS Integer = 0
  FOR EACH cp IN encoding::utf32Encode(text)
    advance = advance + __canvas_glyphAdvance(b, __canvas_glyphIndex(b, cp))
  NEXT
  LET ascent AS Float = toFloat(__canvas_fontAscent(b)) * scale
  ' `hhea.descender` is negative in the file -- it measures downward from the
  ' baseline -- and `TextMetrics.descent` is documented as a positive number, so the
  ' sign is flipped here rather than left for every caller to remember.
  LET descent AS Float = toFloat(0 - __canvas_fontDescent(b)) * scale
  LET lineGap AS Float = toFloat(__canvas_fontLineGap(b)) * scale
  RETURN TextMetrics[width := toFloat(advance) * scale, height := ascent + descent + lineGap, ascent := ascent, descent := descent, lineGap := lineGap]
END FUNC"#;

/// `canvas::fontBytes(font) AS List OF Byte` — an owned copy of the font file.
///
/// **It copies, and the first version did not.** Returning the resource's own block as
/// a read-only alias is the obvious optimisation — a font file is hundreds of
/// kilobytes and `measureText` runs per string per frame, so copying makes measuring
/// cost the font's size rather than the string's length. It is also wrong: the
/// returned value is bound to an ordinary `LET` inside `__canvas_measureText`, and the
/// binding's scope-drop reclaims it, so the *second* call on the same font reads a
/// block the first call freed. Measured, not reasoned: the first `measureText` printed
/// correct metrics and the next one segfaulted (exit 139).
///
/// So the cost is real and paid deliberately, for now. What removes it is a glyph
/// cache — plan-98-G Phase 2 — not an alias: caching the *rasterised glyph* skips the
/// whole read, where aliasing only skips the copy and hands out a dangling block to do
/// it. `canvas::getBytes` copies for the same reason.
pub(crate) fn lower_font_bytes(
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

    let closed = builder.label("canvas_font_bytes_closed");
    let done = builder.label("canvas_font_bytes_done");

    let record_slot = builder.allocate_stack_object("canvas_font_bytes_rec", 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    emit_closed_guard(builder, &record, &closed);

    let source = builder.temporary_vreg();
    let bytes = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&bytes, &source, FONT_BYTES));
    let copy = builder.copy_flat_block(&ParameterType::list_of(ParameterType::Byte), &bytes)?;

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
        text: "canvas.fontBytes".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fontBytes",
        intro: "The font file's bytes, as a read-only alias.",
        desc: "Internal. Returns an owned copy of the font file — an alias would be \
               freed by the caller's scope-drop and leave the resource holding a dead \
               block.",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "font",
                desc: "The font to read.",
                aliases: &[],
                ty: ParameterType::named(super::FONT_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec!["ErrResourceClosed"],
            body: Body::abi_function(lower_font_bytes),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "measureText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "font",
                    desc: "The font to measure in.",
                    aliases: &[],
                    ty: ParameterType::named(super::FONT_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "size",
                    desc: "The em size in pixels.",
                    aliases: &[],
                    ty: ParameterType::Float,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "text",
                    desc: "The string to measure.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named("TextMetrics"),
            errors: vec!["ErrResourceClosed"],
            body: Body::mfb(MEASURE_TEXT, "__canvas_measureText"),
        }],
    });
}
