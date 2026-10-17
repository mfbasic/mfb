//! `canvas::measureText` and the internal `canvas::fontOpenHandle` it reads through.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::builder::*;
use crate::codegen::engine::operand::Operand;
use crate::codegen::error::constants::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryHelper,
    RegistryPackage,
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

`width` is the sum of the glyphs' advance widths, with the font's **pair kerning**
applied: in `"AV"`, a font that tucks the `V` under the `A` measures the pair narrower
than the two letters apart. The kerning comes from the font itself (see
`canvas::loadFont` for which tables are read), and a font without any measures the
plain advances. Ligatures and complex shaping are not done. Measuring and drawing use
the same glyph walk, kerning included, so the width is exactly what a `canvas::Text`
with the same font, size and string draws.

**Measuring in a list of fonts.** Pass a `List OF RES canvas::Font` instead of one font
and each character is measured in the first font in the list that has a glyph for it,
so a string mixing Latin and CJK text measures in `[latinFace, cjkFace]` with every CJK
character taken from the second font. List order is fallback order. A character none of
the fonts has is measured as the first font's missing-glyph box. Each font's characters
are scaled by that font's own design grid, so fonts with different `unitsPerEm` mix
correctly. `ascent` and `descent` are the largest of the fonts that supplied a
character, or the first font's for an empty string, so the line is tall enough for all
of them. `lineGap` is the first font's. Kerning applies only between two characters
from the same font. A one-font list measures exactly as that font alone does. An empty
list raises `ErrInvalidArgument`.

A font with no `head` table measures as all zeroes rather than failing; in a list, it
adds nothing. A closed font is different: measuring it raises `ErrResourceClosed`, and so
does a list with a closed font anywhere in it. Text already in a
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
  ' The same string measured in a list: here a list of one font, which measures exactly
  ' as the font alone. Add more fonts after it and each character is measured in the
  ' first one that has it.
  LET inList AS canvas::TextMetrics = canvas::measureText([face], 24.0, "hello")
  io::print("same width " & toString(inList.width = m.width))
END FUNC
```"#;

/// `canvas::measureText(font, size, text)` — sum the advances, scale the metrics.
///
/// MFBASIC, over the shaped run every text path reads (`__canvas_shapeRun`), so the
/// width is the drawn width by construction. The width is an **Integer** sum of the
/// run's advances in font units, scaled once; the draw paths add scaled Floats glyph by
/// glyph, and the two are not interchangeable to the last bit.
///
/// Each handle comes from `canvas::fontOpenHandle`, which is what keeps a closed font an
/// `ErrResourceClosed` rather than the all-zero metrics an unpublished handle would read
/// as. The single-font form measures the one-element list, so there is one metric
/// computation (plan-162-C), `__canvas_measureHandles`.
#[rustfmt::skip]
const MEASURE_TEXT: &str =
r#"FUNC __canvas_measureText(font AS canvas::Font, size AS Float, text AS String, letterSpacing AS Float) AS TextMetrics
  RETURN __canvas_measureHandles([canvas::fontOpenHandle(font)], size, text, letterSpacing)
END FUNC"#;

/// `canvas::measureText(fonts, size, text)` — the list form (plan-162-C). An empty list is
/// `ErrInvalidArgument` (77050002, spelled as a literal for the reason
/// `__canvas_loadFontBytes` gives); any closed font in it is `ErrResourceClosed`, as the
/// single form raises for its one font.
#[rustfmt::skip]
const MEASURE_TEXT_FONTS: &str =
r#"FUNC __canvas_measureTextFonts(fonts AS List OF RES canvas::Font, size AS Float, text AS String, letterSpacing AS Float) AS TextMetrics
  IF len(fonts) = 0 THEN
    FAIL error(77050002, "measureText needs at least one font")
  END IF
  MUT handles AS List OF Integer = []
  FOR EACH f IN fonts
    handles = collections::append(handles, canvas::fontOpenHandle(f))
  NEXT
  RETURN __canvas_measureHandles(handles, size, text, letterSpacing)
END FUNC"#;

/// The metrics of `text` shaped in the fonts `handles` names, in fallback order.
///
/// The width is summed **per font**: each font's advances as an Integer in its own units,
/// scaled once by its own `size / unitsPerEm`, and the fonts' widths then added. The draw
/// paths add scaled Floats glyph by glyph, and the two are not interchangeable to the
/// last bit, so a one-font list is exactly the old single-font sum. A font with no `head`
/// has no scale and adds nothing.
///
/// `ascent` and `descent` are the largest of the fonts that supplied a glyph — `fonts[0]`
/// for an empty string — so a line holding a fallback font's taller glyphs is tall enough
/// for them. `lineGap` is `fonts[0]`'s. `height` is their sum, as before. A one-element
/// list whose font has no `head` measures all zeroes.
///
/// The decoration lines (plan-162-D) are `fonts[0]`'s: one underline needs one position.
/// Offsets run down from the baseline to the line's top edge. A font that gives no line
/// gets the defaults the plan measured as the median of 638 installed faces: underline
/// 0.10 em below the baseline and 0.05 em thick, strikeout 0.26 em above it at the
/// underline's thickness.
///
/// `spacing` (plan-162-D) adds once per grapheme boundary inside the string — every
/// glyph the run marks `gapAfter` — so `width` grows by `(graphemes - 1) * spacing` and
/// no trailing gap is measured. It is added only when it is not `0.0`, so a zero spacing
/// leaves the width bit-identical.
#[rustfmt::skip]
const MEASURE_HANDLES: &str =
r#"FUNC __canvas_measureHandles(handles AS List OF Integer, size AS Float, text AS String, spacing AS Float) AS TextMetrics
  __canvas_faceCheck(collections::getOr(handles, 0, 0))
  LET run AS List OF Integer = __canvas_shapeRun(handles, text)
  __canvas_shapeDump(run)
  LET fonts AS Integer = len(handles)
  MUT sums AS List OF Integer = []
  MUT used AS List OF Boolean = []
  MUT k AS Integer = 0
  WHILE k < fonts
    sums = collections::append(sums, 0)
    used = collections::append(used, len(run) = 0 AND k = 0)
    k = k + 1
  END WHILE
  MUT gaps AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < len(run)
    gaps = gaps + collections::getOr(run, i + __CANVAS_RUN_GAP, 0)
    LET which AS Integer = collections::getOr(run, i + __CANVAS_RUN_FONT, 0)
    sums = collections::set(sums, which, collections::getOr(sums, which, 0) + collections::getOr(run, i + __CANVAS_RUN_ADVANCE, 0))
    used = collections::set(used, which, TRUE)
    i = i + __CANVAS_RUN_STRIDE
  END WHILE
  MUT width AS Float = 0.0
  MUT ascent AS Float = 0.0
  MUT descent AS Float = 0.0
  MUT measured AS Boolean = FALSE
  k = 0
  WHILE k < fonts
    LET h AS Integer = collections::getOr(handles, k, 0)
    LET upem AS Integer = __canvas_faceMetric(h, __CANVAS_FACE_UPEM)
    IF upem > 0 AND collections::getOr(used, k, FALSE) THEN
      LET scale AS Float = size / toFloat(upem)
      LET part AS Float = toFloat(collections::getOr(sums, k, 0)) * scale
      LET up AS Float = toFloat(__canvas_faceMetric(h, __CANVAS_FACE_ASCENT)) * scale
      ' `hhea.descender` is negative in the file -- it measures downward from the
      ' baseline -- and `TextMetrics.descent` is documented as a positive number, so the
      ' sign is flipped here rather than left for every caller to remember.
      LET down AS Float = toFloat(0 - __canvas_faceMetric(h, __CANVAS_FACE_DESCENT)) * scale
      IF measured THEN
        width = width + part
        ascent = __canvas_maxF(ascent, up)
        descent = __canvas_maxF(descent, down)
      ELSE
        width = part
        ascent = up
        descent = down
        measured = TRUE
      END IF
    END IF
    k = k + 1
  END WHILE
  MUT lineGap AS Float = 0.0
  LET first AS Integer = collections::getOr(handles, 0, 0)
  LET firstUpem AS Integer = __canvas_faceMetric(first, __CANVAS_FACE_UPEM)
  IF firstUpem > 0 THEN
    lineGap = toFloat(__canvas_faceMetric(first, __CANVAS_FACE_LINEGAP)) * (size / toFloat(firstUpem))
  END IF
  IF NOT measured THEN
    RETURN TextMetrics[width := 0.0, height := 0.0, ascent := 0.0, descent := 0.0, lineGap := 0.0, underlineOffset := 0.0, underlineThickness := 0.0, strikeoutOffset := 0.0, strikeoutThickness := 0.0]
  END IF
  MUT underlineOffset AS Float = 0.10 * size
  MUT underlineThickness AS Float = 0.05 * size
  MUT strikeoutOffset AS Float = 0.0 - 0.26 * size
  IF firstUpem > 0 THEN
    LET firstScale AS Float = size / toFloat(firstUpem)
    IF __canvas_faceMetric(first, __CANVAS_FACE_UNDERLINE_SIZE) <> __CANVAS_FACE_UNSET THEN
      underlineOffset = toFloat(0 - __canvas_faceMetric(first, __CANVAS_FACE_UNDERLINE)) * firstScale
      underlineThickness = toFloat(__canvas_faceMetric(first, __CANVAS_FACE_UNDERLINE_SIZE)) * firstScale
    END IF
  END IF
  MUT strikeoutThickness AS Float = underlineThickness
  IF firstUpem > 0 THEN
    LET firstScale AS Float = size / toFloat(firstUpem)
    IF __canvas_faceMetric(first, __CANVAS_FACE_STRIKEOUT_SIZE) <> __CANVAS_FACE_UNSET THEN
      strikeoutOffset = toFloat(0 - __canvas_faceMetric(first, __CANVAS_FACE_STRIKEOUT)) * firstScale
      strikeoutThickness = toFloat(__canvas_faceMetric(first, __CANVAS_FACE_STRIKEOUT_SIZE)) * firstScale
    END IF
  END IF
  IF spacing <> 0.0 AND gaps > 0 THEN
    width = width + toFloat(gaps) * spacing
  END IF
  RETURN TextMetrics[width := width, height := ascent + descent + lineGap, ascent := ascent, descent := descent, lineGap := lineGap, underlineOffset := underlineOffset, underlineThickness := underlineThickness, strikeoutOffset := strikeoutOffset, strikeoutThickness := strikeoutThickness]
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
        implementations: vec![
            Implementation {
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
                    Parameter {
                        name: "letterSpacing",
                        desc: "Extra pixels after every character but the last, as \
                               `canvas::Text.letterSpacing` adds them. Optional; \
                               `0.0` adds none, and a negative value measures \
                               characters closer.",
                        aliases: &[],
                        ty: ParameterType::Float,
                        default: DefaultValue::Fill {
                            type_name: ParameterType::Float,
                            expr: "0.0",
                        },
                    },
                ],
                return_type: ParameterType::named("TextMetrics"),
                errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
                body: Body::mfb(MEASURE_TEXT, "__canvas_measureText"),
            },
            Implementation {
                params: vec![
                    Parameter {
                        name: "fonts",
                        desc: "The fonts to measure in, in fallback order: each character \
                               is measured in the first font that has it. Must not be \
                               empty.",
                        aliases: &[],
                        ty: ParameterType::list_of(ParameterType::res(ParameterType::named(
                            super::FONT_TYPE_ID,
                        ))),
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
                    Parameter {
                        name: "letterSpacing",
                        desc: "Extra pixels after every character but the last, as \
                               `canvas::Text.letterSpacing` adds them. Optional; \
                               `0.0` adds none, and a negative value measures \
                               characters closer.",
                        aliases: &[],
                        ty: ParameterType::Float,
                        default: DefaultValue::Fill {
                            type_name: ParameterType::Float,
                            expr: "0.0",
                        },
                    },
                ],
                return_type: ParameterType::named("TextMetrics"),
                errors: vec!["ErrInvalidArgument", "ErrResourceClosed", "ErrOutOfMemory"],
                body: Body::mfb(MEASURE_TEXT_FONTS, "__canvas_measureTextFonts"),
            },
        ],
    });
    pkg.add_helper(RegistryHelper::always(
        "canvas_measureHandles",
        MEASURE_HANDLES,
    ));
}
