//! plan-176-D: `rt_canvas_font`'s drawing tests: glyph outlines and pens, filled and
//! stroked runs, the GPU oracle, glyph-cache eviction, rotated text and the text build's
//! allocations. The fixture builder and the render family live in the root,
//! `rt_canvas_font.rs`.

use super::*;

/// A `Text` item inside a translated group draws its glyphs **at the offset**
/// (plan-116-G §4.5, **G5**).
///
/// This is the case a fix written for distance fields misses, and it is why G5 asks for
/// the positional reads to be enumerated rather than reasoned about. A glyph is not a
/// distance field: it is a cached coverage bitmap indexed by whole pixels from the run's
/// origin, so "evaluate the distance at `p - offset`" never reaches it. Text in a
/// translated group would then draw at the *untranslated* origin — the glyphs would be
/// perfectly correct and in the wrong place, which no per-pixel comparison of the group's
/// own area would notice.
///
/// The fixture glyph is a filled square at (100,0)-(400,300) in a 1000-unit em, so at
/// size 200 an 'A' is a solid block roughly 60px across. Asserting both that ink IS at
/// the offset and that there is NONE at the unoffset origin is what separates "moved
/// correctly" from "drawn twice" and from "not drawn at all".
#[test]
fn text_inside_a_translated_group_draws_at_the_offset() {
    const SOURCE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("font failed")
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 40.0, y := 120.0], text := "A", fonts := [face], fontSize := 200.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 0)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("label", [label])
  canvas::present([canvas::Group[id := "", position := vector::Float2[x := 400.0, y := 300.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "label"]])
  io::print("done")
END FUNC
"#;
    let frame = run_with_font_frame("canvas_group_text", SOURCE);
    const W: usize = 900;
    let lit = |x: usize, y: usize| -> bool {
        let i = (y * W + x) * 4;
        frame[i] > 128 && frame[i + 1] > 128
    };
    // The run is drawn at (40,120) inside the group, so it lands near (440,420).
    let mut moved = 0;
    let mut origin = 0;
    for y in 0..640usize {
        for x in 0..W {
            if !lit(x, y) {
                continue;
            }
            if x >= 400 && y >= 300 {
                moved += 1;
            } else {
                origin += 1;
            }
        }
    }
    assert!(
        moved > 500,
        "the glyph run did not draw inside the group's translated quadrant: {moved} lit          pixels there. A glyph is a cached bitmap, not a distance field, so the rule          that moves every shape does not reach it unless the run's ORIGIN is moved.",
    );
    assert_eq!(
        origin, 0,
        "{origin} lit pixels outside the translated quadrant — the run was drawn at its          untranslated position as well as (or instead of) the offset one",
    );
}

/// plan-163-C: `Paint.transparency` fades a glyph run's and a stroked run's alphas at
/// build, as it fades a shape's: `t = 0.5` is `T = 128`, and an opaque alpha becomes
/// `(255 * 127 + 127) / 255 = 127`. So a run filled yellow and one stroked white at
/// transparency 0.5 draw byte-identically to the same runs at alpha 127.
#[test]
fn transparency_fades_text_and_stroked_text() {
    let source = |faded: bool| {
        let (fill, stroke, t) = if faded {
            (
                "color::rgb(255, 255, 0)",
                "color::rgb(255, 255, 255)",
                " { transparency := 0.5 }",
            )
        } else {
            (
                "color::rgba(255, 255, 0, 127)",
                "color::rgba(255, 255, 255, 127)",
                " { transparency := 0.0 }",
            )
        };
        format!(
            r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("font failed")
    RETURN
  END TRAP
  LET filled AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 40.0, y := 220.0], text := "A", fonts := [face], fontSize := 200.0, letterSpacing := 0.0, paint := WITH canvas::fill({fill}){t}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET outlined AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 400.0, y := 220.0], text := "A", fonts := [face], fontSize := 200.0, letterSpacing := 0.0, paint := WITH canvas::stroke({stroke}, 6.0){t}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([filled, outlined])
  io::print("done")
END FUNC
"#
        )
    };
    let faded = run_with_font_frame("canvas_text_faded", &source(true));
    let spelled = run_with_font_frame("canvas_text_spelled", &source(false));
    assert!(
        faded == spelled,
        "transparency 0.5 must draw text exactly as alpha 127"
    );
    assert!(
        faded.chunks(4).any(|p| p[0] > 0 && p[1] > 0 && p[2] == 0),
        "the faded filled run drew"
    );
}

#[test]
fn a_glyph_outline_renders_where_its_own_coordinates_put_it() {
    // Glyph 1 is the square `(100,0)..(400,300)` in font units. At `unitsPerEm` 1000
    // and `size` 100 the scale is exactly 0.1, so with the pen at x=100 and the
    // **baseline** at y=200 the ink is `x` 110..140 and `y` 170..200 — the Y flip is
    // the whole reason the top edge is the *smaller* number, since a font's Y grows
    // upward and the surface's grows down.
    let frame = render(
        "canvas_glyph_render",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "A", fonts := [face], fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#,
    );

    let lit: Vec<(usize, usize)> = (0..frame.len() / 4)
        .filter(|i| frame[i * 4] != 0 || frame[i * 4 + 1] != 0 || frame[i * 4 + 2] != 0)
        .map(|i| (i % WIDTH, i / WIDTH))
        .collect();
    assert!(!lit.is_empty(), "the glyph drew nothing at all");
    let min_x = lit.iter().map(|p| p.0).min().unwrap();
    let max_x = lit.iter().map(|p| p.0).max().unwrap();
    let min_y = lit.iter().map(|p| p.1).min().unwrap();
    let max_y = lit.iter().map(|p| p.1).max().unwrap();

    // A pixel is lit when its *centre* is inside the shape, so the covered centres run
    // 110.5..139.5 and 170.5..199.5 — the last whole pixels inside the 30x30 square.
    assert_eq!(
        (min_x, max_x, min_y, max_y),
        (110, 139, 170, 199),
        "glyph ink is not where its own coordinates put it",
    );
    assert_eq!(lit.len(), 30 * 30, "the square is not solid");
    assert_eq!(pixel(&frame, 125, 185), (255, 255, 255, 255), "inside");
    assert_eq!(pixel(&frame, 105, 185), (0, 0, 0, 255), "left of it");
    assert_eq!(pixel(&frame, 125, 160), (0, 0, 0, 255), "above it");
    assert_eq!(
        pixel(&frame, 125, 210),
        (0, 0, 0, 255),
        "below the baseline"
    );
}

/// bug-757: glyph 2 ("B") is a composite of glyph 1 (the square
/// `(100,0)..(400,300)`) placed twice:
/// - once unoffset;
/// - once offset `(0, 400)` at a uniform F2Dot14 scale of 0.5.
///
/// The offset is unscaled, the default. Before the fix a composite drew nothing; the
/// ink must now be the union of both placed squares, and nothing else.
#[test]
fn a_composite_glyph_draws_every_placed_component() {
    fn be16(v: i16) -> [u8; 2] {
        (v as u16).to_be_bytes()
    }
    let mut composite = Vec::new();
    composite.extend(be16(-1)); // numberOfContours: composite
    for bound in [50, 0, 400, 550] {
        composite.extend(be16(bound)); // xMin, yMin, xMax, yMax
    }
    // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES | MORE_COMPONENTS
    composite.extend(be16(0x0001 | 0x0002 | 0x0020));
    composite.extend(be16(1)); // glyphIndex
    composite.extend(be16(0)); // dx
    composite.extend(be16(0)); // dy
                               // ARG_1_AND_2_ARE_WORDS | ARGS_ARE_XY_VALUES | WE_HAVE_A_SCALE, last component
    composite.extend(be16(0x0001 | 0x0002 | 0x0008));
    composite.extend(be16(1));
    composite.extend(be16(0)); // dx
    composite.extend(be16(400)); // dy
    composite.extend(be16(0x2000)); // scale 0.5 in F2Dot14
    let font = truetype_fixture_with(1000, 2, [100, 0, 400, 300], &composite);
    let (frame, _) = render_env_files(
        "canvas_glyph_composite",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("composite.ttf") TRAP(e)
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "B", fonts := [face], fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#,
        &[],
        &[("composite.ttf", font)],
    );
    // Scale 0.1, pen x=100, baseline y=200. The first component is the plain square:
    // x 110..140, y 170..200. The second is the square halved, `(50,0)..(200,150)`,
    // raised by 400 units: x 105..120, y 145..160. Pixel centres inside each
    // rectangle are lit.
    let inside = |x: usize, y: usize| {
        ((110..140).contains(&x) && (170..200).contains(&y))
            || ((105..120).contains(&x) && (145..160).contains(&y))
    };
    let lit: Vec<(usize, usize)> = (0..frame.len() / 4)
        .filter(|i| frame[i * 4] != 0 || frame[i * 4 + 1] != 0 || frame[i * 4 + 2] != 0)
        .map(|i| (i % WIDTH, i / WIDTH))
        .collect();
    assert!(!lit.is_empty(), "the composite glyph drew nothing at all");
    let stray: Vec<_> = lit
        .iter()
        .filter(|(x, y)| !inside(*x, *y))
        .take(5)
        .collect();
    assert!(
        stray.is_empty(),
        "ink outside the two components: {stray:?}"
    );
    assert_eq!(lit.len(), 30 * 30 + 15 * 15, "both components solid");
    assert_eq!(
        pixel(&frame, 125, 185),
        (255, 255, 255, 255),
        "first component"
    );
    assert_eq!(
        pixel(&frame, 112, 152),
        (255, 255, 255, 255),
        "second component"
    );
}

#[test]
fn a_string_advances_the_pen_between_glyphs() {
    // "AA" is the same outline twice, one advance apart. Glyph 1's advance is 250
    // font units = 25 px, so the second square starts 25 px right of the first and the
    // two overlap into one 55-px-wide band — which is what a *monospaced* run looks
    // like when the glyph is wider than its advance, and is exactly the arithmetic the
    // pen has to get right.
    let frame = render(
        "canvas_glyph_advance",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "AA", fonts := [face], fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#,
    );
    let lit: Vec<usize> = (0..frame.len() / 4)
        .filter(|i| frame[i * 4] != 0)
        .map(|i| i % WIDTH)
        .collect();
    assert!(!lit.is_empty(), "the string drew nothing at all");
    assert_eq!(
        (*lit.iter().min().unwrap(), *lit.iter().max().unwrap()),
        (110, 164),
        "the second glyph is not one advance right of the first",
    );
}

/// bug-756 guards: the four text paths — measure, filled run, stroked edges, and the
/// verify reference — all walk the same glyphs with the same pen arithmetic.
///
/// `"ABXA"` exercises every column of that walk: two mapped glyphs with different
/// advances (`A` 250 units, `B` 300), an unmapped `X` that falls to glyph 0 (500), and
/// a repeat, so a walk that dropped a glyph, swapped two, or read an advance from the
/// wrong glyph moves the second `A`. `B` and glyph 0 have no outline, so only the two
/// `A`s draw, and where the second lands is the sum of the three advances before it.
///
/// At size 50 the scale is 0.05, so the advances are 12.5, 15 and 25 px — not whole
/// pixels — and a start of `x = 100.25` puts the pens at 100.25, 112.75, 127.75 and
/// 152.75. The filled run snaps each pen to a whole pixel (`toInt(pen + 0.5)`) and the
/// stroked edges do not, so each path is compared against two single-`A` items at
/// 100.25 and 152.75, which draw through the same path with no walk to get wrong.
const WALK_TEXT: &str = "ABXA";

const WALK_SIZE: &str = "50.0";

const WALK_PENS: [&str; 2] = ["100.25", "152.75"];

fn walk_scene(paint: &str, whole_string: bool) -> String {
    let items = if whole_string {
        format!(
            "[canvas::Text[id := \"\", position := vector::Float2[x := {}, y := 200.0], text := \"{WALK_TEXT}\", fonts := [face], fontSize := {WALK_SIZE}, letterSpacing := 0.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]]",
            WALK_PENS[0]
        )
    } else {
        let one = |x: &str| {
            format!(
                "canvas::Text[id := \"\", position := vector::Float2[x := {x}, y := 200.0], text := \"A\", fonts := [face], fontSize := {WALK_SIZE}, letterSpacing := 0.0, paint := {paint}, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]"
            )
        };
        format!("[{}, {}]", one(WALK_PENS[0]), one(WALK_PENS[1]))
    };
    format!(
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  canvas::present({items})
END FUNC
"#
    )
}

/// Every live font **draws**, however many are loaded. The renderer finds a font's
/// outlines through a process-global table keyed by handle, and that table held
/// sixteen fonts: the seventeenth loaded and measured, and every glyph drawn in it
/// silently came out empty (bug-756). The table is now a chain of sixteen-slot chunks,
/// so forty fonts span three: an `A` in the seventeenth (second chunk) and the fortieth
/// (third) must light exactly what the same two `A`s in the first font light.
#[test]
fn every_live_font_draws_however_many_are_loaded() {
    let scene = |left: &str, right: &str| {
        let mut source = String::from(
            "IMPORT app\nIMPORT canvas\nIMPORT vector\nIMPORT color\n\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n",
        );
        for i in 1..=40 {
            source.push_str(&format!(
                "  RES f{i} AS canvas::Font = canvas::loadFont(\"fixture.ttf\")\n"
            ));
        }
        let text = |x: &str, font: &str| {
            format!(
                "canvas::Text[id := \"\", position := vector::Float2[x := {x}, y := 200.0], text := \"A\", fonts := [{font}], fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]"
            )
        };
        source.push_str(&format!(
            "  canvas::present([{}, {}])\nEND FUNC\n",
            text("100.0", left),
            text("300.0", right)
        ));
        source
    };
    let first = render("canvas_font_first_of_40", &scene("f1", "f1"));
    let later = render("canvas_font_later_of_40", &scene("f17", "f40"));
    assert_eq!(
        lit_column_runs(&first),
        vec![(110, 139), (310, 339)],
        "the first font's `A` is the 30 px square at each pen"
    );
    assert!(
        first == later,
        "text in the 17th and 40th live fonts drew {:?}, not the first font's {:?}",
        lit_column_runs(&later),
        lit_column_runs(&first),
    );
}

/// The filled run places each glyph at its snapped pen: `"ABXA"` lights exactly what
/// two single `A`s at the first and fourth pens light.
#[test]
fn a_filled_run_places_every_glyph_at_its_own_snapped_pen() {
    let fill = "canvas::fill(color::rgb(255, 255, 255))";
    let whole = render("canvas_walk_fill", &walk_scene(fill, true));
    let parts = render("canvas_walk_fill_parts", &walk_scene(fill, false));
    let runs = lit_column_runs(&whole);
    assert_eq!(runs.len(), 2, "two `A`s, two separate bands: {runs:?}");
    assert!(
        whole == parts,
        "the run {runs:?} is not two `A`s at pens {WALK_PENS:?} ({:?})",
        lit_column_runs(&parts),
    );
}

/// The stroked path (`__canvas_textEdges`) lays out the same run with an **unsnapped**
/// pen, and must agree with two stroked single `A`s at the same unsnapped pens.
#[test]
fn a_stroked_run_places_every_glyph_at_its_own_pen() {
    let stroke = "canvas::stroke(color::rgb(255, 255, 255), 2.0)";
    let whole = render("canvas_walk_stroke", &walk_scene(stroke, true));
    let parts = render("canvas_walk_stroke_parts", &walk_scene(stroke, false));
    let runs = lit_column_runs(&whole);
    assert_eq!(runs.len(), 2, "two `A`s, two separate outlines: {runs:?}");
    assert!(
        whole == parts,
        "the stroked run {runs:?} is not two `A`s at pens {WALK_PENS:?} ({:?})",
        lit_column_runs(&parts),
    );
}

/// Verify mode rebuilds the glyph run through `__canvas_geoRunReference` and compares it
/// with the record the renderer drew, so the two walks must agree glyph for glyph.
#[test]
fn verify_mode_agrees_with_the_filled_run() {
    let (_, stats) = render_env(
        "canvas_walk_verify",
        &walk_scene("canvas::fill(color::rgb(255, 255, 255))", true),
        &[("MFB_CANVAS_GEO_VERIFY", "1")],
    );
    assert!(
        stat(&stats, "geoResolvedChecked") >= 1,
        "the text record was never checked:\n{stats}"
    );
    assert_eq!(
        stat(&stats, "geoVerifyMismatches"),
        0,
        "the verify reference walked the run differently from the renderer:\n{stats}"
    );
}

#[test]
fn text_whose_font_was_destroyed_draws_nothing() {
    // The runtime draws empty rather than following a font it cannot resolve. This is
    // the property that lets `canvas::destroyFont` be safe while a scene still names
    // the font, so it is worth pinning separately from the happy path.
    //
    // **Written as destroy-then-present since plan-116-I** (§4.3). It used to fabricate
    // `FontRef[id := 12345]` — a handle naming nothing — which is not expressible now
    // that `Text.font` holds the font itself. That is a strictly better test for the
    // same property: a made-up integer proved the renderer tolerated a bad *number*,
    // while destroying a real font proves it tolerates the case that actually happens,
    // and exercises the `closed` flag the bridge reads rather than a value no resource
    // ever had.
    let frame = render(
        "canvas_glyph_no_font",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 200.0], text := "A", fonts := [face], fontSize := 100.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  ' Destroyed BEFORE the present, so the frame is rendered from a scene whose font is
  ' already gone — the ordering the guard exists for.
  canvas::destroyFont(face)
  canvas::present([label])
END FUNC
"#,
    );
    assert!(
        frame.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0),
        "text whose font was destroyed drew something",
    );
}

/// A text scene the GPU draws — four glyphs, at a size whose bitmaps fit Metal's
/// per-glyph payload.
///
/// Since the glyph cache landed, a glyph reaches the GPU as a *coverage bitmap* rather
/// than as flattened edges, so the old `MAX_EDGES` cap no longer decides whether text is
/// GPU-drawable. What decides it now is bitmap size: Metal carries one glyph's coverage
/// in a `setFragmentBytes:` payload, capped at 4 KiB — about 64x64 — and Vulkan carries
/// the whole frame's in a buffer region. At size 120 the fixture's square is 36x36, well
/// inside both.
const GPU_TEXT: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 300.0], text := "AAAA", fonts := [face], fontSize := 120.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(220, 40, 160)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#;

#[test]
fn text_on_the_gpu_matches_the_software_oracle() {
    let software = render_with("canvas_text_gpu_sw", GPU_TEXT, false);
    let gpu = render_with("canvas_text_gpu_hw", GPU_TEXT, true);
    if gpu.is_empty() {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        software.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    // A tolerance rather than exact equality, for the same reason every other GPU
    // comparison uses one: the shader composites in linear space with hardware blending
    // and the oracle blends in sRGB, so antialiased edges land within a step or two of
    // each other rather than on the same value.
    //
    // What this catches is the failure that is otherwise silent. Before the backends
    // could draw a glyph, Metal accepted a scene whose kind its shader did not know and
    // returned a frame with the text simply missing — 4,536 pixels wrong, reported as
    // success. A GPU frame that merely *resembles* the oracle is the same lie in a
    // quieter form, which is why this compares pixels rather than checking that the
    // renderer claimed the GPU.
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!("GPU text differs from the software oracle: {diff:?}");
    }
}

/// Shapes drawn **after** a glyph run land in the right place, and the glyphs are drawn
/// exactly once.
///
/// plan-116-A made consecutive non-text items one instanced draw. A glyph run ends that
/// run and takes item-buffer slots of its own, so the next run has to restart past them.
/// If it does not, the trailing shapes are drawn as one run *beginning at the first
/// glyph* — and every glyph quad is drawn a second time.
///
/// **The label is translucent, and that is the whole reason this test can see the bug.**
/// The fixture glyph is an axis-aligned opaque square, so its coverage is binary, and
/// compositing an opaque square over itself is idempotent: with an opaque label a
/// renderer that drew every glyph twice produced a byte-identical frame and no assertion
/// could tell. At alpha 150 a second composite is arithmetically different from one.
/// (Measured on the Vulkan harness, which is where this was first caught: bug present →
/// worst channel delta 27; bug absent → 1.)
///
/// The trailing circle is what forces a non-empty final run — with the text last, the
/// final flush is always empty and the ordering is never exercised.
const GPU_TEXT_THEN_SHAPE: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET under AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 80.0, y := 150.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 300.0, height := 200.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 60, 90)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 100.0, y := 300.0], text := "AAAA", fonts := [face], fontSize := 120.0, letterSpacing := 0.0, paint := canvas::fill(color::rgba(220, 40, 160, 150)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET tail AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 200.0], radius := 60.0, paint := canvas::fill(color::rgb(120, 220, 60)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([under, label, tail])
END FUNC
"#;

#[test]
fn a_shape_after_a_glyph_run_matches_the_software_oracle() {
    let software = render_with("canvas_text_tail_sw", GPU_TEXT_THEN_SHAPE, false);
    let gpu = render_with("canvas_text_tail_hw", GPU_TEXT_THEN_SHAPE, true);
    if gpu.is_empty() {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        software.iter().any(|&b| b != 0),
        "the software render drew nothing, so the comparison would be vacuous",
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "a scene of shape, translucent text, shape differs from the software \
             oracle: {diff:?}\n\
             A difference concentrated in the text band means the glyph quads were \
             drawn twice — the instanced run after the glyphs restarted at the wrong \
             base. A difference at the trailing circle means it was not drawn at all."
        );
    }
}

/// Two scenes of three hundred text items each, presented A, B, A.
///
/// Every size is a distinct glyph-cache key (`__canvas_sizeQ` quantises to 1/16 px and
/// the step here is 0.2) and the two scenes' size ranges are disjoint, so the program
/// asks for six hundred distinct glyphs while never needing more than three hundred at
/// once. That is what makes eviction *possible*: a glyph is pinned while any live
/// geometry entry references it, and while the frame being rendered holds its offset —
/// so the only glyphs that can ever be dropped are ones no scene on screen still names.
/// Scene B is what makes scene A's glyphs droppable, and the second A is what proves
/// they come back identical.
///
/// The fixture glyph is a square with four edges, so six hundred rasterisations are
/// cheap while the bitmaps — 6x6 up to 36x36 — add up to far more than the forced budget.
const EVICTION: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections

FUNC scene(face AS canvas::Font, base AS Float) AS List OF canvas::DrawItem
  LET white AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  MUT items AS List OF canvas::DrawItem = []
  MUT i AS Integer = 0
  WHILE i < 300
    LET size AS Float = base + toFloat(i) * 0.2
    LET x AS Float = 4.0 + toFloat(i MOD 20) * 45.0
    LET y AS Float = 36.0 + toFloat(i / 20) * 40.0
    LET glyph AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := x, y := y], text := "A", fonts := [face], fontSize := size, letterSpacing := 0.0, paint := white, transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
    items = collections::append(items, glyph)
    i = i + 1
  END WHILE
  RETURN items
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET a AS List OF canvas::DrawItem = scene(face, 20.0)
  LET b AS List OF canvas::DrawItem = scene(face, 80.0)
  canvas::present(a)
  canvas::present(b)
  ' The frame that gets dumped, and the one that is compared: scene A again, after
  ' scene B has had the chance to push its glyphs out of the cache.
  canvas::present(a)
END FUNC
"#;

#[test]
fn eviction_frees_unpinned_glyphs_and_changes_no_pixel() {
    // Same program, same scene, two budgets. The roomy run never evicts, so it is the
    // oracle; the forced run cannot hold the scene and has to drop and re-raster.
    let (thrashed, pressed) = render_env(
        "canvas_glyph_eviction",
        EVICTION,
        &[("MFB_CANVAS_GLYPH_BUDGET", "8192")],
    );
    let (roomy, relaxed) = render_env("canvas_glyph_roomy", EVICTION, &[]);

    // The pressure is real, and it is *only* pressure — the default budget holds the
    // whole scene, which is what makes eviction a cache behaviour rather than something
    // ordinary text runs into.
    assert!(
        stat(&pressed, "glyphEvictions") > 0,
        "the forced budget never evicted:\n{pressed}",
    );
    assert_eq!(
        stat(&relaxed, "glyphEvictions"),
        0,
        "a 300-item scene evicted at the default budget:\n{relaxed}",
    );

    // Entries really left the cache. Without this the test would still pass if
    // `__canvas_glyphEvict` compacted the cache while keeping every entry — which is
    // exactly what it does when everything is pinned, and is not eviction.
    //
    // The roomy run ends holding all six hundred glyphs; the pressured one cannot, and
    // the ones it dropped are scene B's, which nothing on screen still names.
    let kept = stat(&pressed, "glyphs");
    let all = stat(&relaxed, "glyphs");
    assert_eq!(
        all, 600,
        "the roomy run should still hold both scenes' glyphs",
    );
    assert!(
        kept < all,
        "eviction dropped nothing: {kept} entries under pressure, {all} without",
    );

    // And the pixels are identical. This is the whole claim: a glyph the live scene
    // still draws is never the one dropped, and one that *was* dropped re-rasters to
    // the same bitmap it had before. Exact, not tolerant — a single wrong pixel here
    // means a glyph went missing or came back different, and both are silent failures
    // in a cache that reports nothing.
    let height = (roomy.len() / 4 / WIDTH) as u32;
    let got = Frame::from_rgba(WIDTH as u32, height, thrashed);
    let want = Frame::from_rgba(WIDTH as u32, height, roomy);
    assert!(
        want.pixels.iter().any(|&b| b != 0),
        "the oracle frame is blank, so the comparison would be vacuous",
    );
    if let Err(diff) = compare_exact(&got, &want) {
        panic!("a frame drawn under cache pressure differs from the same frame drawn without it: {diff:?}");
    }
}

/// A rotated text run draws rotated, and where the matrix says.
///
/// plan-116-C §4.5. A glyph's pixels come from a cached coverage bitmap, and under a
/// transform the blit has to invert: walk the surface region the glyph now covers and
/// sample the bitmap, rather than walk the bitmap and write surface pixels. Walking the
/// bitmap under a rotation leaves holes, because the mapping is no longer one sample
/// per pixel.
///
/// The fixture glyph is a solid square, which makes this checkable without judging an
/// antialiased edge: a 90° rotation of a horizontal row of squares is a *vertical*
/// column of squares, so "is there ink here" is a whole-pixel question.
///
/// 90° exactly, so nearest sampling is exact too — the point here is the inverted loop
/// and the transformed per-glyph bounds, not the sampling filter.
const ROTATED_TEXT: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET t AS canvas::Transform = canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 400.0, ty := 80.0]
  LET p AS canvas::Paint = canvas::fill(color::rgb(255, 255, 255))
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 40.0, y := 60.0], text := "AAAA", fonts := [face], fontSize := 60.0, letterSpacing := 0.0, paint := p, transform := t]
  canvas::present([label])
END FUNC
"#;

/// The same run with no transform, so the two can be compared as pictures.
const UPRIGHT_TEXT: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN
  END TRAP
  LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 40.0, y := 60.0], text := "AAAA", fonts := [face], fontSize := 60.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([label])
END FUNC
"#;

#[test]
fn a_rotated_text_run_draws_rotated() {
    let upright = render_with("canvas_text_upright", UPRIGHT_TEXT, false);
    let rotated = render_with("canvas_text_rotated", ROTATED_TEXT, false);
    assert!(
        upright.iter().any(|&b| b != 0),
        "the upright run drew nothing, so the comparison would be vacuous"
    );

    let lit = |frame: &[u8], x: usize, y: usize| {
        frame[(y * WIDTH + x) * 4 + 3] != 0 && frame[(y * WIDTH + x) * 4] != 0
    };
    // Ink in the upright run's own band, which the rotated one must have vacated.
    let mut upright_ink = 0usize;
    let mut rotated_there = 0usize;
    for y in 20..60 {
        for x in 40..240 {
            if lit(&upright, x, y) {
                upright_ink += 1;
            }
            if lit(&rotated, x, y) {
                rotated_there += 1;
            }
        }
    }
    assert!(
        upright_ink > 500,
        "the upright run should fill its band; found {upright_ink} lit pixels"
    );
    assert_eq!(
        rotated_there, 0,
        "the rotated run must have left the upright band entirely — {rotated_there} \
         pixels of it are still lit, so the transform did not move the glyphs"
    );

    // And it must be SOMEWHERE: a transform that dropped the run would also vacate
    // the band, so count the rotated run's own ink and check it matches.
    let total = |frame: &[u8]| {
        (0..WIDTH * (frame.len() / 4 / WIDTH))
            .filter(|i| frame[i * 4 + 3] != 0 && frame[i * 4] != 0)
            .count()
    };
    let (up, rot) = (total(&upright), total(&rotated));
    assert!(
        rot > 0,
        "the rotated run drew nothing at all — the inverted blit found no samples"
    );
    // A 90° rotation is area-preserving, and the glyph is a solid square, so the ink
    // count should match closely. Allow a small margin for the boundary pixels that
    // nearest sampling rounds differently.
    let diff = (up as i64 - rot as i64).abs();
    assert!(
        diff * 20 < up as i64,
        "a 90-degree rotation preserves area, so the rotated run should have about as \
         much ink as the upright one: upright {up}, rotated {rot}"
    );
}

/// A transformed text run reaches the GPU, and draws there what the oracle draws.
///
/// Two claims in one render, because they fail together and are cheap to separate only
/// here:
///
/// 1. **Neither `*Renderable` predicate declines a transform.** `gpuSelected=TRUE` is
///    the observable for that — the renderer sets it only after the predicate let the
///    scene through, so a predicate that grew a `hasTransform` bail would flip the
///    scene to software and this assertion, not the pixel comparison, is what would
///    fail. That distinction is the point: falling back to software still produces a
///    *correct* picture, so the comparison below would pass while the GPU path went
///    entirely unexercised.
/// 2. **The glyph fragment path's inverse sample matches the oracle's.** A transformed
///    run is the one case where the per-glyph quad cannot be narrowed to the glyph's
///    own box (see the `_glyph_hull` block in `runtime/canvas/vulkan/publish.rs`), so this is
///    also the test that the run's transformed hull is what the vertex stage expands.
#[test]
fn a_transformed_text_run_reaches_the_gpu_and_matches_the_oracle() {
    let software = render_with("canvas_text_xform_sw", ROTATED_TEXT, false);
    let (gpu, stats) = render_env(
        "canvas_text_xform_hw",
        ROTATED_TEXT,
        &[("MFB_CANVAS_GPU", "1")],
    );
    if !stats.contains("metalReady=TRUE") {
        eprintln!("skip: this host reports no Metal device");
        return;
    }
    assert!(
        !stats.contains("gpuFrames=0"),
        "a scene whose only item carries a transform was refused by the renderable \
         predicate and fell back to software. The transform slots sit past the header \
         fields the predicates read, so nothing in them should reach either: {stats}"
    );
    assert!(
        software.iter().any(|&b| b != 0),
        "the software render of the rotated run drew nothing, so the comparison would \
         be vacuous"
    );

    let height = (software.len() / 4 / WIDTH) as u32;
    let want = Frame {
        width: WIDTH as u32,
        height,
        pixels: software,
    };
    let got = Frame {
        width: WIDTH as u32,
        height,
        pixels: gpu,
    };
    if let Err(diff) = compare_within_tolerance(&got, &want, Tolerance::GPU_DEFAULT) {
        panic!(
            "the GPU's transformed glyph run disagrees with the oracle: {diff}\n\
             The inverse sample is the suspect: the shader maps the fragment back \
             through the run's inverse and reads the cached coverage there, so a \
             whole-run offset is a wrong translation, and a run that came out sheared \
             or mirrored is a transposed matrix."
        );
    }
}

/// Two hundred presents through one font leave the glyph cache where they found it
/// (plan-116-I Phase 3).
///
/// A `Text` now holds the font itself, so every present publishes a scene containing a
/// resource. What this guards is the **pointer-chase** path — the renderer reaching
/// through the item to the resource record on every frame — rather than a file
/// descriptor: fonts are arena-backed, so a leak here shows up as cache growth.
///
/// **One font, presented two hundred times**, not two hundred loads. That distinction
/// is the whole test. Each `loadFont` mints a *new backend identity*, and the glyph
/// cache is keyed by it, so N loads legitimately produce N cache entries and a loop
/// written that way measures the cache's key, not the pointer chase. Measured: 200
/// loads leave `glyphs=200 glyphBytes=22000 glyphEvictions=0` — bounded by the cache
/// cap rather than leaking, but growing for a reason that has nothing to do with this
/// letter (**I6**).
#[test]
fn two_hundred_presents_through_one_font_leave_the_glyph_cache_where_they_found_it() {
    let (_, stats) = render_env(
        "canvas_font_present_churn",
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load-failed")
    RETURN
  END TRAP
  MUT i AS Integer = 0
  WHILE i < 200
    ' The x moves each frame so the scene differs and the present is not skipped —
    ' a skipped present would exercise nothing.
    LET label AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 10.0 + toFloat(i), y := 40.0], text := "A", fonts := [face], fontSize := 24.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
    canvas::present([label])
    i = i + 1
  END WHILE
END FUNC
"#,
        &[],
    );
    assert!(
        !stats.contains("load-failed"),
        "the loop never loaded a font: {stats}"
    );
    let glyphs = stat(&stats, "glyphs");
    let bytes = stat(&stats, "glyphBytes");
    assert!(
        glyphs <= 4 && bytes <= 4096,
        "200 presents through ONE font left glyphs={glyphs} glyphBytes={bytes}. One \
         face drawing one character at one size is a single cache entry; growth \
         proportional to the number of PRESENTS means the renderer retains something \
         each time it chases the item's pointer to the resource: {stats}"
    );
}

// --- bug-1091: a text build reads the font in place -----------------------------------

/// The graphics thread's arena's `alloc_bytes`, from a `--debug` report on `stderr`: the
/// arena whose `kind` is `graphics`.
fn graphics_alloc_bytes(stderr: &str) -> u64 {
    let index = stderr
        .lines()
        .find_map(|line| {
            let rest = line.strip_prefix("arena.")?;
            let (index, kind) = rest.split_once(".kind ")?;
            (kind.trim() == "graphics").then(|| index.to_string())
        })
        .unwrap_or_else(|| panic!("no graphics arena in the report:\n{stderr}"));
    let key = format!("arena.{index}.alloc_bytes ");
    stderr
        .lines()
        .find_map(|line| line.strip_prefix(key.as_str()))
        .unwrap_or_else(|| panic!("no `{key}` in the report:\n{stderr}"))
        .trim()
        .parse()
        .expect("a byte count")
}

/// 200 presents, each one filled and one stroked `Text` at a new position, so every
/// present rebuilds both records: the graphics arena's `alloc_bytes` for that run.
fn text_rebuild_alloc_bytes(name: &str, font: Vec<u8>) -> u64 {
    let (stdout, stderr, project) = run_debug_files(
        name,
        r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("load-failed")
    RETURN
  END TRAP
  MUT i AS Integer = 0
  WHILE i < 200
    LET filled AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 10.0 + toFloat(i), y := 40.0], text := "AB", fonts := [face], fontSize := 24.0, letterSpacing := 0.0, paint := canvas::fill(color::rgb(255, 255, 255)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
    LET stroked AS canvas::DrawItem = canvas::Text[id := "", position := vector::Float2[x := 10.0 + toFloat(i), y := 120.0], text := "AB", fonts := [face], fontSize := 24.0, letterSpacing := 0.0, paint := canvas::stroke(color::rgb(255, 255, 255), 2.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
    canvas::present([filled, stroked])
    i = i + 1
  END WHILE
  io::print("done")
END FUNC
"#,
        &[("fixture.ttf", font)],
        &[
            ("MFB_CANVAS_SYNC", "1".to_string()),
            ("MFB_CANVAS_GPU", "0".to_string()),
        ],
    );
    let _ = std::fs::remove_dir_all(&project);
    assert_eq!(stdout.trim(), "done", "the program did not draw: {stderr}");
    graphics_alloc_bytes(&stderr)
}

/// Building a `Text` record does not cost the font file's size (bug-1091).
///
/// Every text build used to fetch an owned copy of the whole file
/// (`canvas::fontBlobUnchecked`) for the outlines, though every glyph after the first
/// frame is a cache hit: one copy per filled build and one per stroked build, 400 here.
/// Measured before the fix: padding the fixture with a 1 MB table nothing reads put
/// 419,436,800 more bytes through the graphics arena (466,485,120 against 885,921,920),
/// 400 copies of the 1,048,880-byte file. The difference is the file's cost, so it is
/// what is bounded: less than one copy across all 400 builds. A ratio would not see it,
/// because each frame's 2.3 MB surface dwarfs one copy.
#[test]
fn a_text_build_does_not_copy_the_font_file() {
    let small = text_rebuild_alloc_bytes("canvas_font_build_small", minimal_truetype());
    let padded = text_rebuild_alloc_bytes(
        "canvas_font_build_padded",
        with_table(&minimal_truetype(), b"zpad", vec![0u8; 1 << 20]),
    );
    assert!(
        padded.saturating_sub(small) < 1 << 20,
        "200 presents of a filled and a stroked text allocated {small} graphics-thread \
         bytes with the 304-byte fixture and {padded} with it padded to 1 MB: the builds \
         copy the font file"
    );
}
