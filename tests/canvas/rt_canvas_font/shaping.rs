//! plan-176-D: `rt_canvas_font`'s font lists, shaped runs, letter spacing, decoration
//! metrics, caret stops and ligatures. The shared sfnt/OpenType table builders live in
//! the root, `rt_canvas_font.rs`.

use super::*;

// ---------------------------------------------------------------------------------------
// plan-162-C: a list of fonts, in fallback order.
//
// Two fixtures with disjoint `cmap`s: the "latin" one is the kerning base font without
// kerning (`A` → 1, advance 250; `V` → 2, advance 300; em 1000; ascent 800, descent 200,
// lineGap 100), and the "fallback" one maps `B` → 1 (advance 250) and `C` → 2 (advance
// 300) in a 2000-unit em with ascent 1800, descent 600 and lineGap 50 — so at size 1000
// its glyphs are half as wide per unit and its line is taller. Its glyphs' outline is a
// 1600 × 600 block, where the latin font's is 300 × 300, so the two draw differently.

/// The fallback fixture: `B` and `C`, em 2000, a taller line.
fn fallback_font() -> Vec<u8> {
    // A wider square than the latin font's, so a glyph drawn from it is told apart by
    // its pixels: 0..1600 units, which is 0..80 px at size 100.
    let square = [0, 0, 1600, 600];
    let base = truetype_fixture_with(2000, 2, square, &square_record(square));
    let cmap = cmap_with(3, 10, cmap12(&[(b'B' as u32, b'C' as u32, 1)]));
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&be16(1800));
    hhea[6..8].copy_from_slice(&be16((-600i16) as u16));
    hhea[8..10].copy_from_slice(&be16(50));
    hhea[34..36].copy_from_slice(&be16(3));
    with_tables(&base, &[(b"cmap", cmap), (b"hhea", hhea)])
}

/// Run `body` (statements inside `main`, with `latin` and `fallback` loaded) and return
/// what it printed.
fn run_two_fonts(name: &str, body: &str) -> Vec<String> {
    let source = format!(
        r#"IMPORT app
IMPORT canvas
IMPORT encoding
IMPORT io

' A close behind a resource parameter consumes nothing at the caller, so the caller can
' still name the font, and using it raises at run time (`.ai/canvas-threading.md` R3b).
FUNC closeIt(f AS canvas::Font) AS Nothing
  canvas::destroyFont(f)
END FUNC

FUNC thousandths(m AS canvas::TextMetrics) AS String
  RETURN toString(toInt(m.width * 1000.0)) & " " & toString(toInt(m.ascent * 1000.0)) & " " & toString(toInt(m.descent * 1000.0)) & " " & toString(toInt(m.lineGap * 1000.0)) & " " & toString(toInt(m.height * 1000.0))
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES latin AS canvas::Font = canvas::loadFont("latin.ttf")
  RES fallback AS canvas::Font = canvas::loadFont("fallback.ttf")
{body}
END FUNC
"#
    );
    run_files(
        name,
        &source,
        &[
            ("latin.ttf", kerning_font(&[])),
            ("fallback.ttf", fallback_font()),
        ],
    )
}

/// The width is summed per font in that font's own units and scale: `A` and `V` from
/// the first font (250 + 300 units at scale 1), `B` from the second (250 units at scale
/// 0.5), and `X`, which neither maps, is the first font's glyph 0 (500 units). Ascent and
/// descent are the largest of the fonts that supplied a glyph; lineGap is the first
/// font's.
#[test]
fn measure_text_over_a_font_list_sums_each_font_at_its_own_scale() {
    let lines = run_two_fonts(
        "canvas_measure_list_mixed",
        r#"  io::print(thousandths(canvas::measureText([latin, fallback], 1000.0, "AVB")))
  io::print(thousandths(canvas::measureText([latin, fallback], 1000.0, "AV")))
  io::print(thousandths(canvas::measureText([latin, fallback], 1000.0, "X")))
  io::print(thousandths(canvas::measureText([fallback, latin], 1000.0, "BA")))
  io::print(thousandths(canvas::measureText([latin, fallback], 1000.0, "")))"#,
    );
    assert_eq!(
        lines,
        vec![
            // 550 + 125 wide; the fallback's 900 / 300 line box; the latin lineGap.
            "675000 900000 300000 100000 1300000",
            // Only the first font supplied a glyph: its own box.
            "550000 800000 200000 100000 1100000",
            // Neither maps `X`: the first font's glyph 0, 500 units.
            "500000 800000 200000 100000 1100000",
            // Order is fallback order: `B` from the first-listed font, `A` from the
            // second; the line gap is the first font's, 50 units at scale 0.5.
            "375000 900000 300000 25000 1225000",
            // An empty string measures the first font's line.
            "0 800000 200000 100000 1100000",
        ],
    );
}

/// `measureText([f], …)` is `measureText(f, …)`, field for field, and an empty list or a
/// closed font in the list raises what the single form would.
#[test]
fn measure_text_over_a_one_font_list_is_the_single_font_form() {
    let lines = run_two_fonts(
        "canvas_measure_list_single",
        r#"  io::print(thousandths(canvas::measureText([latin], 37.0, "AVAV")) & " / " & thousandths(canvas::measureText(latin, 37.0, "AVAV")))
  io::print(thousandths(canvas::measureText([fallback], 13.0, "BCX")) & " / " & thousandths(canvas::measureText(fallback, 13.0, "BCX")))
  LET none AS List OF RES canvas::Font = []
  LET empty AS canvas::TextMetrics = canvas::measureText(none, 10.0, "A") TRAP(e)
    io::print("empty: " & toString(e.code))
    RECOVER canvas::measureText(latin, 10.0, "A")
  END TRAP
  closeIt(fallback)
  LET closed AS canvas::TextMetrics = canvas::measureText([latin, fallback], 10.0, "A") TRAP(e)
    io::print("closed: " & toString(e.code))
    RECOVER canvas::measureText(latin, 10.0, "A")
  END TRAP"#,
    );
    let same = |line: &String| {
        let (a, b) = line.split_once(" / ").expect("two measurements");
        a == b
    };
    assert_eq!(lines.len(), 4, "{lines:?}");
    assert!(
        same(&lines[0]) && same(&lines[1]),
        "a one-font list must measure as the font: {lines:?}"
    );
    assert_eq!(
        lines[2], "empty: 77050002",
        "an empty list is ErrInvalidArgument"
    );
    assert_eq!(
        lines[3], "closed: 77030004",
        "a closed font in the list is ErrResourceClosed"
    );
}

/// The lit column runs of `text` drawn at size 100 from x = 100 in `fonts`, a list over
/// the bindings `latin` and `fallback`. `close_latin` closes the first behind a resource
/// parameter before drawing.
fn two_font_columns(name: &str, text: &str, fonts: &str, close_latin: bool) -> Vec<(usize, usize)> {
    let close = if close_latin {
        "  closeIt(latin)\n"
    } else {
        ""
    };
    let source = format!(
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC closeIt(f AS canvas::Font) AS Nothing
  canvas::destroyFont(f)
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES latin AS canvas::Font = canvas::loadFont("latin.ttf")
  RES fallback AS canvas::Font = canvas::loadFont("fallback.ttf")
{close}  canvas::present([canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "{text}", fonts := {fonts}, fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])
END FUNC
"#
    );
    let (frame, _) = render_env_files(
        name,
        &source,
        &[],
        &[
            ("latin.ttf", kerning_font(&[])),
            ("fallback.ttf", fallback_font()),
        ],
    );
    lit_column_runs(&frame)
}

/// Each glyph draws from the first font that has it. `A` is the latin font's square,
/// columns 110..139 (100..400 units at 0.1); `B`, which only the fallback has, starts
/// at the pen after `A` (250 units → 125) and is the fallback's 0..1600-unit block at 0.05:
/// columns 125..204. Drawn from the latin font instead, `B` would be its `.notdef` —
/// no outline — and the ink would end at 139.
#[test]
fn a_two_font_string_draws_each_glyph_from_its_own_font() {
    let both = two_font_columns("canvas_fallback_draw", "AB", "[latin, fallback]", false);
    assert_eq!(
        both,
        vec![(110, 204)],
        "`A` from the latin font, `B` from the fallback"
    );
    let latin_only = two_font_columns("canvas_fallback_draw_one", "AB", "[latin]", false);
    assert_eq!(
        latin_only,
        vec![(110, 139)],
        "with one font `B` is its empty `.notdef`"
    );
}

/// A closed font is passed over: with the latin font closed, `A` is the fallback's
/// `.notdef` (no outline, advance 500 units → 25 px) and `B` draws from the fallback at
/// pen 125: columns 125..204, and nothing at the latin square's 110..139.
#[test]
fn closing_the_first_font_draws_the_string_from_the_next() {
    let runs = two_font_columns("canvas_fallback_closed", "AB", "[latin, fallback]", true);
    assert_eq!(runs, vec![(125, 204)]);
}

// ---------------------------------------------------------------------------------------
// plan-162-D: the five-column shaped run — `scalarIndex` and `gapAfter` — and what is
// built on it. A `--debug` build run with `MFB_CANVAS_SHAPE_DUMP=<file>` appends every run
// `measureText` shapes as `shape` and its words.

/// The run's columns, as `__CANVAS_RUN_*` number them.
const RUN_STRIDE: usize = 6;

const RUN_SCALAR: usize = 3;

const RUN_GAP: usize = 4;

/// Shape each codepoint list in `cases` (with `measureText`), and return, per case, the
/// program's own `strings::graphemes` count, its scalar count, and the dumped run.
fn shaped_runs(name: &str, cases: &[&[u32]]) -> Vec<(usize, usize, Vec<i64>)> {
    let mut body = String::new();
    for cps in cases {
        let list: Vec<String> = cps.iter().map(|c| c.to_string()).collect();
        body.push_str(&format!(
            "  probe(f, encoding::utf32Decode([{}]))\n",
            list.join(", ")
        ));
    }
    let source = format!(
        r#"IMPORT app
IMPORT canvas
IMPORT encoding
IMPORT strings
IMPORT io

FUNC probe(f AS canvas::Font, s AS String) AS Nothing
  io::print(toString(len(strings::graphemes(s))) & " " & toString(len(encoding::utf32Encode(s))))
  LET m AS canvas::TextMetrics = canvas::measureText(f, 10.0, s)
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
{body}END FUNC
"#
    );
    let (stdout, _, project) = run_debug_files(
        name,
        &source,
        &[("fixture.ttf", minimal_truetype())],
        &[("MFB_CANVAS_SHAPE_DUMP", "{project}/shape.txt".to_string())],
    );
    let dump = std::fs::read_to_string(project.join("shape.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    let counts: Vec<(usize, usize)> = stdout
        .lines()
        .map(|l| {
            let (g, s) = l.split_once(' ').expect("two counts");
            (g.parse().expect("a count"), s.parse().expect("a count"))
        })
        .collect();
    let runs: Vec<Vec<i64>> = dump
        .lines()
        .map(|l| {
            l.split(' ')
                .skip(1)
                .map(|w| w.parse().expect("a run word"))
                .collect()
        })
        .collect();
    assert_eq!(
        counts.len(),
        cases.len(),
        "one count line per case:\n{stdout}"
    );
    assert_eq!(runs.len(), cases.len(), "one shaped run per case:\n{dump}");
    counts
        .into_iter()
        .zip(runs)
        .map(|((g, s), r)| (g, s, r))
        .collect()
}

/// The run marks grapheme boundaries exactly as `strings::graphemes` draws them: one
/// `gapAfter` per boundary inside the string (so `graphemes − 1` of them, none after the
/// last), on the last glyph of each grapheme; and `scalarIndex` walks the string's
/// scalars in order, ending at the last. The cases are the ones a naive "one gap per
/// codepoint" or "per space" rule gets wrong: combining marks, a ZWJ family, CRLF,
/// regional-indicator flags, Hangul jamo, an emoji with a skin-tone modifier, a
/// Devanagari conjunct, a lone combining mark at the start, and the empty string.
#[test]
fn shape_run_marks_every_grapheme_boundary_and_scalar() {
    let cases: &[&[u32]] = &[
        &[101, 0x301, 120],                                          // é (e + acute), x
        &[0x1F469, 0x200D, 0x1F469, 0x200D, 0x1F467, 97],            // family ZWJ, a
        &[65, 13, 10, 66],                                           // A CRLF B
        &[0x1F1EF, 0x1F1F5, 0x1F1FA, 0x1F1F8],                       // two flags
        &[0x1100, 0x1161, 0x11A8, 0x1100],                           // Hangul LVT, L
        &[72, 0x308, 0x1F44D, 0x1F3FD, 13, 10, 0x915, 0x94D, 0x937], // mixed
        &[0x301, 65],                                                // lone mark first
        &[],                                                         // empty
    ];
    for (i, (graphemes, scalars, run)) in shaped_runs("canvas_shape_run_graphemes", cases)
        .into_iter()
        .enumerate()
    {
        assert_eq!(
            run.len() % RUN_STRIDE,
            0,
            "case {i}: whole records: {run:?}"
        );
        let glyphs = run.len() / RUN_STRIDE;
        assert_eq!(
            glyphs, scalars,
            "case {i}: one glyph per scalar until ligatures"
        );
        let gaps = (0..glyphs)
            .filter(|g| run[g * RUN_STRIDE + RUN_GAP] == 1)
            .count();
        assert_eq!(
            gaps,
            graphemes.saturating_sub(1),
            "case {i}: one gap per inner grapheme boundary: {run:?}"
        );
        if glyphs > 0 {
            assert_eq!(
                run[(glyphs - 1) * RUN_STRIDE + RUN_GAP],
                0,
                "case {i}: no gap after the last grapheme"
            );
            let scalar = |g: usize| run[g * RUN_STRIDE + RUN_SCALAR];
            assert!(
                (1..glyphs).all(|g| scalar(g) >= scalar(g - 1)),
                "case {i}: scalarIndex must not go backwards: {run:?}"
            );
            assert_eq!(
                scalar(glyphs - 1),
                scalars as i64 - 1,
                "case {i}: ends on the last scalar"
            );
        }
    }
}

/// The rightmost lit column of `"AV"` drawn at size 100 from x = 100 in the unkerned
/// two-glyph font, presented once per spacing in `spacings` (the dump is the last frame).
fn spaced_right_edge(name: &str, paint: &str, spacings: &[&str]) -> usize {
    let presents: String = spacings
        .iter()
        .map(|s| format!("  canvas::present([canvas::Text[id := \"\", position := vector::Float2[x := 100.0, y := 200.0], text := \"AV\", fonts := [face], fontSize := 100.0, letterSpacing := {s}, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\n"))
        .collect();
    let source = format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\n\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n  RES face AS canvas::Font = canvas::loadFont(\"plain.ttf\")\n{presents}END FUNC\n"
    );
    let (frame, _) = render_env_files(name, &source, &[], &[("plain.ttf", kerning_font(&[]))]);
    let runs = lit_column_runs(&frame);
    assert!(!runs.is_empty(), "{name}: `AV` drew nothing");
    runs.last().unwrap().1
}

/// `letterSpacing` moves the second grapheme by exactly the spacing, on the filled run
/// and the stroked edges, positive or negative.
#[test]
fn letter_spacing_moves_the_next_character_by_the_spacing() {
    for (paint, label) in [
        ("canvas::fill(color::rgb(255, 255, 255))", "fill"),
        ("canvas::stroke(color::rgb(255, 255, 255), 2.0)", "stroke"),
    ] {
        let plain = spaced_right_edge(&format!("canvas_spacing_plain_{label}"), paint, &["0.0"]);
        let wide = spaced_right_edge(&format!("canvas_spacing_wide_{label}"), paint, &["7.0"]);
        let tight = spaced_right_edge(
            &format!("canvas_spacing_tight_{label}"),
            paint,
            &["0.0 - 5.0"],
        );
        assert_eq!(wide, plain + 7, "{label}: +7 px of spacing");
        assert_eq!(tight + 5, plain, "{label}: -5 px of spacing");
    }
}

/// A change of spacing alone, between two presents of an otherwise identical `Text`,
/// draws the new spacing: the cache key folds it, so the run is rebuilt.
#[test]
fn a_spacing_only_change_redraws() {
    let fill = "canvas::fill(color::rgb(255, 255, 255))";
    let fresh = spaced_right_edge("canvas_spacing_fresh", fill, &["9.0"]);
    let changed = spaced_right_edge("canvas_spacing_changed", fill, &["0.0", "9.0"]);
    assert_eq!(
        changed, fresh,
        "the second present must draw at the new spacing"
    );
}

/// `measureText`'s spacing adds once per grapheme boundary, never after the last:
/// `"AVA"` (250 + 300 + 250 units) is three graphemes, so `+10.0` adds 20 px and `-4.0`
/// takes 8; a combining mark joins its base, so `A` + U+0301 (250 + `.notdef`'s 500) is
/// one grapheme and spacing adds nothing to it.
#[test]
fn measure_text_spacing_adds_between_graphemes_only() {
    let lines = run_two_fonts(
        "canvas_measure_spacing",
        r#"  io::print(toString(toInt(canvas::measureText(latin, 1000.0, "AVA").width)))
  io::print(toString(toInt(canvas::measureText(latin, 1000.0, "AVA", 10.0).width)))
  io::print(toString(toInt(canvas::measureText([latin, fallback], 1000.0, "AVA", 0.0 - 4.0).width)))
  io::print(toString(toInt(canvas::measureText(latin, 1000.0, "A" & encoding::utf32Decode([769]), 10.0).width)))
  io::print(toString(toInt(canvas::measureText(latin, 1000.0, "A" & encoding::utf32Decode([769])).width)))"#,
    );
    assert_eq!(lines, vec!["800", "820", "792", "750", "750"]);
}

/// A `post` table with `underlinePosition` and `underlineThickness`, and an `OS/2` table
/// with `yStrikeoutSize` and `yStrikeoutPosition` (the only `OS/2` fields read).
fn decorated_font(underline: (i16, i16), strikeout: (i16, i16)) -> Vec<u8> {
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    post[8..10].copy_from_slice(&underline.0.to_be_bytes());
    post[10..12].copy_from_slice(&underline.1.to_be_bytes());
    let mut os2 = vec![0u8; 96];
    os2[0..2].copy_from_slice(&be16(4));
    os2[26..28].copy_from_slice(&strikeout.1.to_be_bytes());
    os2[28..30].copy_from_slice(&strikeout.0.to_be_bytes());
    with_tables(&minimal_truetype(), &[(b"OS/2", os2), (b"post", post)])
}

/// The four decoration metrics at size 1000 in a 1000-unit em (scale 1), in pixels:
/// `underlineOffset underlineThickness strikeoutOffset strikeoutThickness`.
fn decoration(name: &str, font: Vec<u8>) -> String {
    let lines = run_files(
        name,
        r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("fixture.ttf")
  LET m AS canvas::TextMetrics = canvas::measureText(f, 1000.0, "A")
  io::print(toString(toInt(m.underlineOffset)) & " " & toString(toInt(m.underlineThickness)) & " " & toString(toInt(m.strikeoutOffset)) & " " & toString(toInt(m.strikeoutThickness)))
END FUNC
"#,
        &[("fixture.ttf", font)],
    );
    lines.join("\n")
}

/// The font's own lines, measured down from the baseline to each line's top edge:
/// `post.underlinePosition` −150 is 150 px below, 60 thick; `OS/2.yStrikeoutPosition`
/// 280 is 280 px above, 45 thick. A font with neither table gets the defaults — 0.10 em
/// below at 0.05 em, and 0.26 em above at the underline's thickness — and a zero
/// thickness counts as no line at all.
#[test]
fn decoration_metrics_come_from_post_and_os2_or_the_defaults() {
    assert_eq!(
        decoration(
            "canvas_decoration_font",
            decorated_font((-150, 60), (280, 45))
        ),
        "150 60 -280 45"
    );
    assert_eq!(
        decoration("canvas_decoration_none", minimal_truetype()),
        "100 50 -260 50"
    );
    assert_eq!(
        decoration(
            "canvas_decoration_zero",
            decorated_font((-150, 0), (280, 0))
        ),
        "100 50 -260 50",
        "a zero thickness is no line"
    );
    assert_eq!(
        decoration(
            "canvas_decoration_underline_only",
            decorated_font((-120, 70), (280, 0))
        ),
        "120 70 -260 70",
        "no strikeout: the default position, at the font's underline thickness"
    );
}

/// 50 strings over `A`, `V` (a kerned pair in the gpos1 fixture), a space (`.notdef`),
/// and `A` + U+0301 (two scalars, one grapheme), each with one of three spacings. For
/// every one, `caretStops`' last `x` is `measureText`'s `width` exactly, and its indices
/// are exactly the grapheme starts `strings::graphemes` gives, plus the length.
#[test]
fn caret_stops_agree_with_measuring_and_with_graphemes() {
    let mut seed: u32 = 162;
    let mut next = || {
        seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        (seed >> 16) as usize
    };
    let pieces: [&[u32]; 4] = [&[65], &[86], &[32], &[65, 0x301]];
    let spacings = ["0.0", "3.5", "0.0 - 1.25"];
    let mut body = String::new();
    for _ in 0..50 {
        let len = next() % 7;
        let mut cps: Vec<String> = Vec::new();
        for _ in 0..len {
            for cp in pieces[next() % pieces.len()] {
                cps.push(cp.to_string());
            }
        }
        let spacing = spacings[next() % spacings.len()];
        body.push_str(&format!(
            "  check(f, encoding::utf32Decode([{}]), {spacing})\n",
            cps.join(", ")
        ));
    }
    let source = format!(
        r#"IMPORT app
IMPORT canvas
IMPORT encoding
IMPORT strings
IMPORT io

FUNC check(f AS canvas::Font, s AS String, spacing AS Float) AS Nothing
  LET stops AS List OF canvas::CaretStop = canvas::caretStops(f, 37.0, s, spacing)
  LET width AS Float = canvas::measureText(f, 37.0, s, spacing).width
  MUT want AS String = "0"
  MUT at AS Integer = 0
  FOR EACH g IN strings::graphemes(s)
    at = at + len(encoding::utf32Encode(g))
    want = want & "," & toString(at)
  NEXT
  MUT got AS String = ""
  MUT lastX AS Float = 0.0 - 1.0
  FOR EACH stop IN stops
    IF len(got) > 0 THEN
      got = got & ","
    END IF
    got = got & toString(stop.index)
    lastX = stop.x
  NEXT
  io::print(toString(lastX = width) & " " & toString(got = want))
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("kerned.ttf")
{body}END FUNC
"#
    );
    let lines = run_files(
        "canvas_caret_stops_agree",
        &source,
        &[("kerned.ttf", kerning_font(&[(b"GPOS", gpos_format1())]))],
    );
    assert_eq!(lines.len(), 50);
    for (i, line) in lines.iter().enumerate() {
        assert_eq!(
            line, "TRUE TRUE",
            "string {i}: last stop = width, indices = grapheme starts"
        );
    }
}

/// Stops sit where the characters after them are drawn: in the gpos1 fixture at size
/// 1000, `AV` kerns to 250 − 120 = 130 for `A`, so the stop before `V` is at 130, and the
/// end at 130 + 300. An empty string is one stop, `0` at 0; a closed font raises
/// `ErrResourceClosed` and an empty list `ErrInvalidArgument`.
#[test]
fn caret_stops_place_kerned_characters_and_refuse_closed_fonts() {
    let lines = run_files(
        "canvas_caret_stops_edges",
        r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC closeIt(f AS canvas::Font) AS Nothing
  canvas::destroyFont(f)
END FUNC

FUNC show(stops AS List OF canvas::CaretStop) AS String
  MUT out AS String = ""
  FOR EACH stop IN stops
    out = out & toString(stop.index) & ":" & toString(toInt(stop.x)) & " "
  NEXT
  RETURN out
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("kerned.ttf")
  RES g AS canvas::Font = canvas::loadFont("kerned.ttf")
  io::print(show(canvas::caretStops(f, 1000.0, "AV")))
  io::print(show(canvas::caretStops([g, f], 1000.0, "AV", 10.0)))
  io::print(show(canvas::caretStops(f, 1000.0, "")))
  LET none AS List OF RES canvas::Font = []
  LET a AS List OF canvas::CaretStop = canvas::caretStops(none, 10.0, "A") TRAP(e)
    io::print("empty: " & toString(e.code))
    RECOVER []
  END TRAP
  closeIt(g)
  LET b AS List OF canvas::CaretStop = canvas::caretStops([g, f], 10.0, "A") TRAP(e)
    io::print("closed: " & toString(e.code))
    RECOVER []
  END TRAP
END FUNC
"#,
        &[("kerned.ttf", kerning_font(&[(b"GPOS", gpos_format1())]))],
    );
    assert_eq!(
        lines,
        vec![
            "0:0 1:130 2:430 ",
            "0:0 1:140 2:440 ",
            "0:0 ",
            "empty: 77050002",
            "closed: 77030004",
        ]
    );
}

// ---------------------------------------------------------------------------------------
// plan-162-F: standard ligatures (`GSUB` `liga`).
//
// The ligature fixture: `f` → 1 (advance 300, a 100 × 300 bar), `i` → 2 (200, 80 × 200),
// `l` → 3 (250, 80 × 400), and two ligature glyphs no codepoint maps: `fi` → 4 (450, a
// 400 × 100 slab) and `ffi` → 5 (700, a 650 × 100 slab), so a drawn ligature is told
// apart from its components by its pixels. `liga` lists `ffi` before `fi`, longest
// first, as fonts do. `GDEF` gives `ffi` carets at 230 and 470; `fi` has none.

const GID_F: u16 = 1;

const GID_I: u16 = 2;

const GID_L: u16 = 3;

const GID_FI: u16 = 4;

const GID_FFI: u16 = 5;

/// LigatureSubst format 1 over `firsts`, one ligature set each: `(ligature, rest)`,
/// where `rest` is the components after the first.
fn ligature_subst(firsts: &[u16], sets: &[Vec<(u16, Vec<u16>)>]) -> Vec<u8> {
    let mut set_bytes: Vec<Vec<u8>> = Vec::new();
    for set in sets {
        let mut ligs: Vec<Vec<u8>> = Vec::new();
        for (lig, rest) in set {
            let mut b = Vec::new();
            b.extend(be16(*lig));
            b.extend(be16(rest.len() as u16 + 1));
            for g in rest {
                b.extend(be16(*g));
            }
            ligs.push(b);
        }
        let mut b = Vec::new();
        b.extend(be16(ligs.len() as u16));
        let mut at = 2 + 2 * ligs.len();
        for l in &ligs {
            b.extend(be16(at as u16));
            at += l.len();
        }
        for l in ligs {
            b.extend(l);
        }
        set_bytes.push(b);
    }
    let header = 6 + 2 * sets.len();
    let mut out = Vec::new();
    out.extend(be16(1));
    let total: usize = set_bytes.iter().map(Vec::len).sum();
    out.extend(be16((header + total) as u16)); // coverage after the sets
    out.extend(be16(sets.len() as u16));
    let mut at = header;
    for b in &set_bytes {
        out.extend(be16(at as u16));
        at += b.len();
    }
    for b in set_bytes {
        out.extend(b);
    }
    out.extend(coverage1(firsts));
    out
}

/// A `GDEF` with only a LigCaretList: per ligature glyph, format-1 caret coordinates.
fn gdef_carets(ligatures: &[(u16, Vec<i16>)]) -> Vec<u8> {
    let mut glyphs: Vec<Vec<u8>> = Vec::new();
    for (_, carets) in ligatures {
        let mut b = Vec::new();
        b.extend(be16(carets.len() as u16));
        let mut at = 2 + 2 * carets.len();
        for _ in carets {
            b.extend(be16(at as u16));
            at += 4;
        }
        for c in carets {
            b.extend(be16(1));
            b.extend(c.to_be_bytes());
        }
        glyphs.push(b);
    }
    let firsts: Vec<u16> = ligatures.iter().map(|(g, _)| *g).collect();
    let cov = coverage1(&firsts);
    let mut list = Vec::new();
    let header = 4 + 2 * glyphs.len();
    list.extend(be16(header as u16)); // coverage right after the offsets
    list.extend(be16(glyphs.len() as u16));
    let mut at = header + cov.len();
    for g in &glyphs {
        list.extend(be16(at as u16));
        at += g.len();
    }
    list.extend(cov);
    for g in glyphs {
        list.extend(g);
    }
    let mut out = Vec::new();
    out.extend(be16(1));
    out.extend(be16(0)); // version 1.0
    out.extend(be16(0)); // glyphClassDef
    out.extend(be16(0)); // attachList
    out.extend(be16(12)); // ligCaretList
    out.extend(be16(0)); // markAttachClassDef
    out.extend(list);
    out
}

/// The ligature fixture, with `extra` tables (a `GSUB`, a `GDEF`) added.
fn ligature_font(extra: &[(&[u8; 4], Vec<u8>)]) -> Vec<u8> {
    let outlines: [[i16; 4]; 5] = [
        [0, 0, 100, 300],
        [0, 0, 80, 200],
        [0, 0, 80, 400],
        [0, 0, 400, 100],
        [0, 0, 650, 100],
    ];
    let mut glyf = Vec::new();
    let mut loca = vec![0u16, 0]; // glyph 0: no outline
    for o in outlines {
        glyf.extend(square_record(o));
        loca.push((glyf.len() / 2) as u16);
    }
    let loca: Vec<u8> = loca.iter().flat_map(|v| v.to_be_bytes()).collect();
    let mut hmtx = Vec::new();
    for advance in [500u16, 300, 200, 250, 450, 700] {
        hmtx.extend(be16(advance));
        hmtx.extend(be16(0));
    }
    let mut hhea = vec![0u8; 36];
    hhea[4..6].copy_from_slice(&be16(800));
    hhea[6..8].copy_from_slice(&be16((-200i16) as u16));
    hhea[34..36].copy_from_slice(&be16(6));
    let cmap = cmap_with(
        3,
        10,
        cmap12(&[
            (b'f' as u32, b'f' as u32, GID_F as u32),
            (b'i' as u32, b'i' as u32, GID_I as u32),
            (b'l' as u32, b'l' as u32, GID_L as u32),
        ]),
    );
    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
    ];
    tables.extend(extra.iter().map(|(t, d)| (*t, d.clone())));
    with_tables(&minimal_truetype(), &tables)
}

/// `liga`: on `f`, `ffi` (f, f, i) before `fi` (f, i).
fn gsub_liga() -> Vec<u8> {
    let set = vec![(GID_FFI, vec![GID_F, GID_I]), (GID_FI, vec![GID_I])];
    layout_table(
        b"DFLT",
        b"liga",
        &[lookup(4, &[ligature_subst(&[GID_F], &[set])])],
    )
}

fn ligating_font() -> Vec<u8> {
    ligature_font(&[
        (b"GSUB", gsub_liga()),
        (b"GDEF", gdef_carets(&[(GID_FFI, vec![230, 470])])),
    ])
}

/// Widths at size 1000 in a 1000-unit em, so in font units: `ffi` is the one `ffi`
/// glyph (700), `fil` is `fi` + `l` (450 + 250), and with letter-spacing there are no
/// ligatures, so `fi` is `f` + `i` + the one gap (300 + 200 + 1).
#[test]
fn liga_measures_ligatures_and_not_with_spacing() {
    let lines = run_files(
        "canvas_liga_widths",
        r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("liga.ttf")
  io::print(toString(toInt(canvas::measureText(f, 1000.0, "ffi").width)))
  io::print(toString(toInt(canvas::measureText(f, 1000.0, "fil").width)))
  io::print(toString(toInt(canvas::measureText(f, 1000.0, "fi", 1.0).width)))
END FUNC
"#,
        &[("liga.ttf", ligating_font())],
    );
    assert_eq!(lines, vec!["700", "700", "501"]);
}

/// The lit column runs of `text` drawn at size 100 from x = 100 in the ligature font.
fn liga_columns(name: &str, text: &str, spacing: &str) -> Vec<(usize, usize)> {
    let source = format!(
        "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\n\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n  RES face AS canvas::Font = canvas::loadFont(\"liga.ttf\")\n  canvas::present([canvas::Text[id := \"\", position := vector::Float2[x := 100.0, y := 200.0], text := \"{text}\", fonts := [face], fontSize := 100.0, letterSpacing := {spacing}, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]])\nEND FUNC\n"
    );
    let (frame, _) = render_env_files(name, &source, &[], &[("liga.ttf", ligating_font())]);
    lit_column_runs(&frame)
}

/// The drawn `fi` is the one `fi` glyph — a 400-unit slab, columns 100..139 at 0.1 — not
/// an `f` bar (100..109) and an `i` bar after it. With spacing on, it is those two.
#[test]
fn liga_draws_the_ligature_glyph() {
    assert_eq!(
        liga_columns("canvas_liga_draw", "fi", "0.0"),
        vec![(100, 139)]
    );
    assert_eq!(
        liga_columns("canvas_liga_draw_spaced", "fi", "1.0"),
        vec![(100, 109), (131, 138)],
        "spacing turns the ligature off: f at 100, i at 100 + 30 + 1"
    );
}

/// Carets inside a ligature: `ffi` has `GDEF` carets at 230 and 470, so its stops are
/// there; `fi` has none, so its one inner stop is half its 450 advance. The run carries
/// the ligature glyph with its component count, after which `l` follows.
#[test]
fn liga_carets_split_the_ligature() {
    let lines = run_debug_files(
        "canvas_liga_carets",
        r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC show(stops AS List OF canvas::CaretStop) AS String
  MUT out AS String = ""
  FOR EACH stop IN stops
    out = out & toString(stop.index) & ":" & toString(toInt(stop.x)) & " "
  NEXT
  RETURN out
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("liga.ttf")
  io::print(show(canvas::caretStops(f, 1000.0, "ffi")))
  io::print(show(canvas::caretStops(f, 1000.0, "fil")))
  io::print(toString(toInt(canvas::measureText(f, 1000.0, "ffil").width)))
END FUNC
"#,
        &[("liga.ttf", ligating_font())],
        &[("MFB_CANVAS_SHAPE_DUMP", "{project}/shape.txt".to_string())],
    );
    let (stdout, _, project) = lines;
    let dump = std::fs::read_to_string(project.join("shape.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        vec!["0:0 1:230 2:470 3:700 ", "0:0 1:225 2:450 3:700 ", "950"]
    );
    // The `ffil` run: the `ffi` ligature (glyph 5, advance 700, scalar 0, a gap after
    // it, 3 components), then `l` (glyph 3, 250, scalar 3, no gap, 1 component).
    let last = dump.lines().last().expect("a shaped run");
    assert_eq!(last, "shape 0 5 700 0 1 3 0 3 250 3 0 1");
}

/// A shipping font ligates as HarfBuzz does. `SFCamera.ttf` carries `liga` as `GSUB`
/// type-4 lookups, and `office fluffy` forms two ligatures in it. The expected glyph ids
/// and width are HarfBuzz 12.2.0's, from `hb-shape --features=liga,kern,-clig,-calt
/// --no-glyph-names --no-clusters /System/Library/Fonts/SFCamera.ttf "office fluffy"` →
/// glyphs 176 147 233 129 137 2 234 202 147 147 218. The width is the glyphs' `hmtx`
/// advances plus HarfBuzz's kerning, kern on minus kern off (the tracking HarfBuzz also
/// applies from `trak` cancels in the difference; plan-162-B Correction 6): 11158 units
/// at size 2048, the face's em. The run's glyph column is read through
/// `MFB_CANVAS_SHAPE_DUMP`. Skipped, logged, when the file is missing or a different
/// build of the font.
#[cfg(target_os = "macos")]
#[test]
fn a_real_font_ligates_as_harfbuzz_does() {
    const PATH: &str = "/System/Library/Fonts/SFCamera.ttf";
    const SHA256: &str = "1c89c9953b2ade5ecef874ff42885dc86beb27d5cd47f52d1759c7f7fece82fc";
    let digest = Command::new("shasum")
        .args(["-a", "256", PATH])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    if !digest.starts_with(SHA256) {
        eprintln!("skipping: {PATH} is missing or not the recorded build (`{digest}`)");
        return;
    }
    let (stdout, _, project) = run_debug_files(
        "canvas_liga_sfcamera",
        r#"IMPORT app
IMPORT canvas
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES f AS canvas::Font = canvas::loadFont("/System/Library/Fonts/SFCamera.ttf")
  io::print(toString(toInt(canvas::measureText(f, 2048.0, "office fluffy").width * 1000.0)))
END FUNC
"#,
        &[],
        &[("MFB_CANVAS_SHAPE_DUMP", "{project}/shape.txt".to_string())],
    );
    let dump = std::fs::read_to_string(project.join("shape.txt")).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(stdout.trim(), "11158000");
    let words: Vec<i64> = dump
        .split_whitespace()
        .skip(1)
        .map(|w| w.parse().expect("a run word"))
        .collect();
    let glyphs: Vec<i64> = words.chunks(RUN_STRIDE).map(|g| g[1]).collect();
    assert_eq!(
        glyphs,
        vec![176, 147, 233, 129, 137, 2, 234, 202, 147, 147, 218]
    );
}
