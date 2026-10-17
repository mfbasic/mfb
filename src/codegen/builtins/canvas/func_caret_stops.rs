//! `canvas::caretStops` and its `canvas::CaretStop` record (plan-162-D).

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RecordProp, RegistryFunction, RegistryHelper,
    RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

const INTRO: &str = r#"The x position of every place a text cursor can stand in a string."#;

const DESC: &str = r#"`caretStops` returns where a text cursor (a caret) can stand in `text` when it is
drawn at `size` in `font` — or in a list of fonts, in fallback order, exactly as
`canvas::measureText` and `canvas::Text` take them. There is one `canvas::CaretStop` per
place between characters, where a character is what a reader sees as one: a letter
with its accents, a whole emoji sequence, a CRLF line break. Stops are in order, and
include both ends: the first is `index` 0 at `x` 0.0, and the last is the string's
length in characters at `x` equal to `canvas::measureText(…).width` for the same
arguments.

`index` is a position in the string in Unicode scalars, the unit `strings::mid`,
`strings::left` and `strings::find` count in, so `strings::left(text, stop.index)` is the
text before the stop. `x` is in pixels from the start of the text — add the `Text`
item's `position.x` to place it on the canvas. Each stop's `x` is where the character
starting there is drawn: kerning and `letterSpacing` are included, so a caret drawn at
a stop sits exactly at the left edge of the character after it. Inside a ligature,
where one glyph draws several characters (`fi` in many fonts), there is still a stop
between them: at the position the font gives for that boundary if it gives one, and
otherwise at an equal share of the ligature's width.

Clicking a position to the nearest stop is a search over `x`, shown below. Stops run
left to right along the line; right-to-left text is not supported.

`letterSpacing` is optional and must match the `Text` item's, or the stops drift from
what is drawn. An empty string has one stop, `index` 0 at `x` 0.0. An empty list of
fonts raises `ErrInvalidArgument`, and a closed font raises `ErrResourceClosed`, as
`canvas::measureText` does. `ErrOutOfMemory` means the program ran out of memory."#;

const EX: &str = r#"```
IMPORT app
IMPORT canvas
IMPORT io
IMPORT strings

' Where a click at `x` puts the caret: the index of the nearest stop.
FUNC nearestStop(stops AS List OF canvas::CaretStop, x AS Float) AS Integer
  MUT best AS Integer = 0
  MUT bestDistance AS Float = 1000000.0
  FOR EACH stop IN stops
    MUT distance AS Float = stop.x - x
    IF distance < 0.0 THEN
      distance = 0.0 - distance
    END IF
    IF distance < bestDistance THEN
      best = stop.index
      bestDistance = distance
    END IF
  NEXT
  RETURN best
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("DejaVuSans.ttf")
  LET text AS String = "AB"
  LET stops AS List OF canvas::CaretStop = canvas::caretStops(face, 24.0, text)
  FOR EACH stop IN stops
    io::print(toString(stop.index) & " at " & toString(stop.x))
  NEXT
  LET at AS Integer = nearestStop(stops, 13.0)
  io::print("a click at 13 puts the caret after \"" & strings::left(text, at) & "\"")
END FUNC
```"#;

/// `canvas::caretStops(font, …)` — the stops of the one-element list.
#[rustfmt::skip]
const CARET_STOPS: &str =
r#"FUNC __canvas_caretStops(font AS canvas::Font, size AS Float, text AS String, letterSpacing AS Float) AS List OF CaretStop
  RETURN __canvas_caretStopsHandles([canvas::fontOpenHandle(font)], size, text, letterSpacing)
END FUNC"#;

/// `canvas::caretStops(fonts, …)` — an empty list is `ErrInvalidArgument` (77050002), and
/// a closed font `ErrResourceClosed`, as `measureText`'s list form raises.
#[rustfmt::skip]
const CARET_STOPS_FONTS: &str =
r#"FUNC __canvas_caretStopsFonts(fonts AS List OF RES canvas::Font, size AS Float, text AS String, letterSpacing AS Float) AS List OF CaretStop
  IF len(fonts) = 0 THEN
    FAIL error(77050002, "caretStops needs at least one font")
  END IF
  MUT handles AS List OF Integer = []
  FOR EACH f IN fonts
    handles = collections::append(handles, canvas::fontOpenHandle(f))
  NEXT
  RETURN __canvas_caretStopsHandles(handles, size, text, letterSpacing)
END FUNC"#;

/// The stops of `text` shaped in `handles`: a projection of the one shaped run, so they
/// agree with drawing by construction.
///
/// The pen walks the run as the draw paths do — each advance at its font's scale, then
/// `spacing` after a `gapAfter` glyph — and a stop lands after every `gapAfter` glyph, at
/// the next glyph's scalar index. A ligature (plan-162-F) stands for several codepoints,
/// so a grapheme can start inside it: such a stop lands at the font's `GDEF` caret for
/// that component boundary, or, for a ligature without carets, at an even share of its
/// advance. The last stop is the scalar count at `__canvas_measureHandles`'s width
/// itself, so it equals `measureText`'s to the bit rather than to the rounding of a
/// second sum.
#[rustfmt::skip]
const CARET_STOPS_HANDLES: &str =
r#"FUNC __canvas_caretStopsHandles(handles AS List OF Integer, size AS Float, text AS String, spacing AS Float) AS List OF CaretStop
  LET run AS List OF Integer = __canvas_shapeRun(handles, text, spacing)
  LET scales AS List OF Float = __canvas_fontScales(handles, size)
  MUT stops AS List OF CaretStop = [CaretStop[index := 0, x := 0.0]]
  LET glyphs AS Integer = len(run) / __CANVAS_RUN_STRIDE
  IF glyphs = 0 THEN
    RETURN stops
  END IF
  LET n AS Integer = len(encoding::utf32Encode(text))
  LET boundary AS List OF Boolean = __canvas_graphemeEnds(text, n)
  MUT pen AS Float = 0.0
  MUT c AS Integer = 0
  WHILE c < glyphs
    LET at AS Integer = c * __CANVAS_RUN_STRIDE
    LET which AS Integer = collections::getOr(run, at + __CANVAS_RUN_FONT, 0)
    LET scale AS Float = collections::getOr(scales, which, 0.0)
    LET advance AS Float = toFloat(collections::getOr(run, at + __CANVAS_RUN_ADVANCE, 0)) * scale
    LET components AS Integer = collections::getOr(run, at + __CANVAS_RUN_COMPONENTS, 1)
    IF components > 1 THEN
      ' Graphemes that start inside a ligature.
      LET first AS Integer = collections::getOr(run, at + __CANVAS_RUN_SCALAR, 0)
      LET gid AS Integer = collections::getOr(run, at + __CANVAS_RUN_GLYPH, 0)
      MUT k AS Integer = 1
      WHILE k < components
        IF collections::getOr(boundary, first + k, FALSE) THEN
          LET caret AS Integer = __canvas_faceCaret(collections::getOr(handles, which, 0), gid, k)
          MUT x AS Float = pen + advance * toFloat(k) / toFloat(components)
          IF caret <> __CANVAS_FACE_UNSET THEN
            x = pen + toFloat(caret) * scale
          END IF
          stops = collections::append(stops, CaretStop[index := first + k, x := x])
        END IF
        k = k + 1
      END WHILE
    END IF
    pen = pen + advance
    IF c < glyphs - 1 AND collections::getOr(run, at + __CANVAS_RUN_GAP, 0) = 1 THEN
      IF spacing <> 0.0 THEN
        pen = pen + spacing
      END IF
      stops = collections::append(stops, CaretStop[index := collections::getOr(run, at + __CANVAS_RUN_STRIDE + __CANVAS_RUN_SCALAR, 0), x := pen])
    END IF
    c = c + 1
  END WHILE
  LET width AS Float = __canvas_measureHandles(handles, size, text, spacing).width
  RETURN collections::append(stops, CaretStop[index := len(encoding::utf32Encode(text)), x := width])
END FUNC"#;

fn params(first: Parameter) -> Vec<Parameter> {
    vec![
        first,
        Parameter {
            name: "size",
            desc: "The em size in pixels, as the `Text` item's `fontSize`.",
            aliases: &[],
            ty: ParameterType::Float,
            default: DefaultValue::None,
        },
        Parameter {
            name: "text",
            desc: "The string, as the `Text` item draws it.",
            aliases: &[],
            ty: ParameterType::String,
            default: DefaultValue::None,
        },
        Parameter {
            name: "letterSpacing",
            desc: "The `Text` item's `letterSpacing`. Optional; `0.0` adds none.",
            aliases: &[],
            ty: ParameterType::Float,
            default: DefaultValue::Fill {
                type_name: ParameterType::Float,
                expr: "0.0",
            },
        },
    ]
}

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_record(RegistryRecord {
        name: "CaretStop",
        export: true,
        description: "One place a text cursor can stand, as `canvas::caretStops` \
                      returns it.",
        props: vec![
            RecordProp {
                name: "index",
                ty: ParameterType::Integer,
                description: "Where the stop is in the string, in Unicode scalars: the \
                              number of scalars before it, as `strings::mid` counts.",
            },
            RecordProp {
                name: "x",
                ty: ParameterType::Float,
                description: "Pixels from the start of the text to the stop: the left \
                              edge of the character after it.",
            },
        ],
    });
    let stops = ParameterType::list_of(ParameterType::named("CaretStop"));
    pkg.add_function(RegistryFunction {
        name: "caretStops",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![
            Implementation {
                params: params(Parameter {
                    name: "font",
                    desc: "The font the text is drawn in.",
                    aliases: &[],
                    ty: ParameterType::named(super::FONT_TYPE_ID),
                    default: DefaultValue::None,
                }),
                return_type: stops.clone(),
                errors: vec!["ErrResourceClosed", "ErrOutOfMemory"],
                body: Body::mfb(CARET_STOPS, "__canvas_caretStops"),
            },
            Implementation {
                params: params(Parameter {
                    name: "fonts",
                    desc: "The fonts the text is drawn in, in fallback order, as the \
                           `Text` item's `fonts`. Must not be empty.",
                    aliases: &[],
                    ty: ParameterType::list_of(ParameterType::res(ParameterType::named(
                        super::FONT_TYPE_ID,
                    ))),
                    default: DefaultValue::None,
                }),
                return_type: stops,
                errors: vec!["ErrInvalidArgument", "ErrResourceClosed", "ErrOutOfMemory"],
                body: Body::mfb(CARET_STOPS_FONTS, "__canvas_caretStopsFonts"),
            },
        ],
    });
    pkg.add_helper(RegistryHelper::always(
        "canvas_caretStopsHandles",
        CARET_STOPS_HANDLES,
    ));
}
