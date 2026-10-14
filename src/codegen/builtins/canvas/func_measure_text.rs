//! `canvas::measureText` and the internal `canvas::fontOpenHandle` it reads through.

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

const INTRO: &str = r#"Measure a string in a font without drawing it."#;

const DESC: &str = r#"`measureText` returns the `canvas::TextMetrics` a `canvas::Text` item would occupy: the advance
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

A font with no `head` table measures as all zeroes rather than failing. A closed
font is different: measuring it raises `ErrResourceClosed`. Text already in a
presented scene still draws (as nothing) after its font is closed; only
measurement needs the font open.

`ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("DejaVuSans.ttf")
  LET m AS canvas::TextMetrics = canvas::measureText(face, 24.0, "hello")
  io::print("width " & toString(m.width) & " height " & toString(m.height))
END FUNC
```"#;

/// `canvas::measureText(font, size, text)` — sum the advances, scale the metrics.
///
/// MFBASIC, over the shaped run every text path reads (`__canvas_shapeRun`), so the
/// width is the drawn width by construction. The width is an **Integer** sum of the
/// run's advances in font units, scaled once; the draw paths add scaled Floats glyph by
/// glyph, and the two are not interchangeable to the last bit.
///
/// The handle comes from `canvas::fontOpenHandle`, which is what keeps a closed font an
/// `ErrResourceClosed` rather than the all-zero metrics an unpublished handle would read
/// as. The `head`/`hhea` metrics come from the font's face index by that handle
/// (`__canvas_faceMetric`, plan-162-A), so measuring copies nothing of the font file.
#[rustfmt::skip]
const MEASURE_TEXT: &str =
r#"FUNC __canvas_measureText(font AS canvas::Font, size AS Float, text AS String) AS TextMetrics
  LET handle AS Integer = canvas::fontOpenHandle(font)
  __canvas_faceCheck(handle)
  LET upem AS Integer = __canvas_faceMetric(handle, __CANVAS_FACE_UPEM)
  IF upem <= 0 THEN
    RETURN TextMetrics[width := 0.0, height := 0.0, ascent := 0.0, descent := 0.0, lineGap := 0.0]
  END IF
  LET scale AS Float = size / toFloat(upem)
  LET run AS List OF Integer = __canvas_shapeRun([handle], text)
  MUT advance AS Integer = 0
  MUT i AS Integer = 2
  WHILE i < len(run)
    advance = advance + collections::getOr(run, i, 0)
    i = i + 3
  END WHILE
  LET ascent AS Float = toFloat(__canvas_faceMetric(handle, __CANVAS_FACE_ASCENT)) * scale
  ' `hhea.descender` is negative in the file -- it measures downward from the
  ' baseline -- and `TextMetrics.descent` is documented as a positive number, so the
  ' sign is flipped here rather than left for every caller to remember.
  LET descent AS Float = toFloat(0 - __canvas_faceMetric(handle, __CANVAS_FACE_DESCENT)) * scale
  LET lineGap AS Float = toFloat(__canvas_faceMetric(handle, __CANVAS_FACE_LINEGAP)) * scale
  RETURN TextMetrics[width := toFloat(advance) * scale, height := ascent + descent + lineGap, ascent := ascent, descent := descent, lineGap := lineGap]
END FUNC"#;

/// `canvas::fontOpenHandle(font) AS Integer` — the font's handle, or `ErrResourceClosed`.
///
/// `canvas::fontHandle` answers `0` for a closed font, because the renderer must draw a
/// scene naming one as nothing rather than stop the frame. Measuring is different: the
/// man page promises `ErrResourceClosed`, and `0` would read as a font with no tables and
/// measure as zeroes. So this is the same closed-before-handle read with the other
/// answer. It replaced `canvas::fontBytes`, which raised the same way but copied the
/// whole font file to hand back (bug-756): text code now asks by handle, and only the
/// readers that need the file fetch it.
pub(crate) fn lower_font_open_handle(
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

    let closed = builder.label("canvas_font_open_closed");
    let done = builder.label("canvas_font_open_done");

    let record_slot = builder.allocate_stack_object("canvas_font_open_rec", 8);
    builder.emit(abi::store_u64(&record, abi::stack_pointer(), record_slot));
    emit_closed_guard(builder, &record, &closed);

    let source = builder.temporary_vreg();
    let handle = builder.temporary_vreg();
    builder.emit(abi::load_u64(&source, abi::stack_pointer(), record_slot));
    builder.emit(abi::load_u64(&handle, &source, RESOURCE_OFFSET_HANDLE));
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &handle));
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
        text: "canvas.fontOpenHandle".to_string(),
    })
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fontOpenHandle",
        intro: "The handle of an open font.",
        desc: "Internal. Raises `ErrResourceClosed` for a closed font, where \
               `canvas::fontHandle` answers `0`.",
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
            return_type: ParameterType::Integer,
            errors: vec!["ErrResourceClosed"],
            body: Body::abi_function(lower_font_open_handle),
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
            errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
            body: Body::mfb(MEASURE_TEXT, "__canvas_measureText"),
        }],
    });
}
