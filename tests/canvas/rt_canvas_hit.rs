//! Hit-testing and `canvas::MouseEvent.overID` (plan-172-I).
//!
//! Every case here runs a real headless `--app` program and asks the public question:
//! a press is injected at a pixel with `MFB_MOUSE_INJECT`, `canvas::pollMouse` returns
//! it, and the test reads `overID`. There is no probe into the hit test — what a
//! program can observe is the contract, and a shape with an `id` reports that `id`
//! exactly when it is hit, so "hit" and "miss" are both visible from outside.
//!
//! Each expectation is worked out by hand from the rules in `mfb spec app canvas`
//! ("Hit-testing"). An event's `position` is a whole pixel and the point tested is its
//! centre, so a press at `(x, y)` tests `(x + 0.5, y + 0.5)`; the scenes below put
//! their edges where that half pixel cannot decide a case.
//!
//! `MFB_MOUSE_INJECT` is POSIX-only (`src/codegen/term/core/term.rs:emit_mouse_inject`),
//! so this file is too. The Windows proof of `overID` is plan-172-M's, on a real box.
#![cfg(unix)]

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// The twelve-glyph fixture font `tests/canvas/rt_canvas_font.rs:minimal_truetype`
/// builds, byte for byte (`scripts/remote-common.sh:rc_fixture_font` ships the same
/// file to the remote boxes): `unitsPerEm` 1000, ascender 800, descender -200, `A`
/// advancing 250 and `B` 300, and one square glyph whose ink is (100, 0)-(400, 300).
/// At `fontSize` 100 the string `AB` is 55 px wide, 80 px above its baseline and 20
/// below, while its ink is two 30 px squares sitting on the baseline.
const FIXTURE_TTF: &[u8] = &[
    0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 99, 109, 97, 112, 0, 0, 0, 0, 0, 0, 0, 108, 0, 0, 0, 52,
    103, 108, 121, 102, 0, 0, 0, 0, 0, 0, 0, 160, 0, 0, 0, 34, 104, 101, 97, 100, 0, 0, 0, 0, 0, 0,
    0, 194, 0, 0, 0, 54, 104, 104, 101, 97, 0, 0, 0, 0, 0, 0, 0, 248, 0, 0, 0, 36, 104, 109, 116,
    120, 0, 0, 0, 0, 0, 0, 1, 28, 0, 0, 0, 12, 108, 111, 99, 97, 0, 0, 0, 0, 0, 0, 1, 40, 0, 0, 0,
    8, 0, 0, 0, 1, 0, 3, 0, 10, 0, 0, 0, 12, 0, 12, 0, 0, 0, 0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 2, 0,
    0, 0, 65, 0, 0, 0, 65, 0, 0, 0, 1, 0, 0, 0, 66, 0, 0, 0, 66, 0, 0, 0, 2, 0, 1, 0, 100, 0, 0, 1,
    144, 1, 44, 0, 3, 0, 0, 1, 1, 1, 1, 0, 100, 1, 44, 0, 0, 254, 212, 0, 0, 0, 0, 1, 44, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 232, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 32, 255, 56,
    0, 100, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3, 1, 244,
    0, 0, 0, 250, 0, 0, 1, 44, 0, 0, 0, 0, 0, 0, 0, 17, 0, 17,
];

/// The helpers every scene below is written with. Each builds one item with an `id`;
/// the paint is beside the point for a hit test and is the same grey throughout.
const PRELUDE: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io
IMPORT vector

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC grey() AS canvas::Paint
  RETURN canvas::fill(color::rgb(200, 200, 200))
END FUNC

FUNC rect(name AS String, x AS Float, y AS Float, w AS Float, h AS Float) AS canvas::DrawItem
  MUT r AS canvas::Rectangle
  r = WITH r { id := name, position := at(x, y), size := canvas::Size[width := w, height := h], paint := grey() }
  LET item AS canvas::DrawItem = r
  RETURN item
END FUNC

FUNC node(name AS String, group AS String, x AS Float, y AS Float) AS canvas::DrawItem
  MUT g AS canvas::Group
  g = WITH g { id := name, name := group, position := at(x, y) }
  LET item AS canvas::DrawItem = g
  RETURN item
END FUNC

' Every injected press, in order, as the bracketed `overID` it reported.
FUNC report(tag AS String) AS Nothing
  canvas::enableMouse(TRUE)
  MUT line AS String = tag
  MUT n AS Integer = 0
  WHILE n < 70
    LET e AS canvas::MouseEvent = canvas::pollMouse()
    IF e.kind <> canvas::MouseKind.None THEN
      line = line & " [" & e.overID & "]"
    END IF
    n = n + 1
  END WHILE
  canvas::enableMouse(FALSE)
  io::print(line)
END FUNC
"#;

/// Build `PRELUDE` + `body` as a canvas program whose `main` is `body`, run it headless
/// with `inject` fed to the mouse decoder, and return its stdout.
///
/// `canvas::enableMouse(TRUE)` feeds the whole injected string through the decoder
/// each time it is called, so a program that enables twice sees every event twice.
fn run(name: &str, body: &str, inject: &str) -> String {
    let source = format!(
        "{PRELUDE}\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n{body}END FUNC\n"
    );
    let project = common::temp_project(name, &source);
    std::fs::write(project.join("fixture.ttf"), FIXTURE_TTF).expect("write the font");
    let binary = common::build_app_debug(&project, name);
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_MOUSE_INJECT", inject)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let stdout = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        run.status.success(),
        "program {}:\n{stdout}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stderr),
    );
    let _ = std::fs::remove_dir_all(&project);
    stdout
}

/// An SGR-1006 press at pixel `(x, y)`: the wire is 1-based column;row, and `code` is
/// the button with its modifier bits (4 is Shift).
fn press(code: u32, x: u32, y: u32) -> String {
    format!("\x1b[<{code};{};{}M", x + 1, y + 1)
}

/// [`run`] with one press of button 0 injected at each of `points`, returning what
/// each `report(tag)` call printed after its tag: one `overID` per press, in order. A
/// program that reports twice answers every press twice — once against each scene it
/// had presented by then.
fn over(name: &str, body: &str, points: &[(u32, u32)]) -> Vec<Vec<String>> {
    let inject: String = points.iter().map(|(x, y)| press(0, *x, *y)).collect();
    let stdout = run(name, body, &inject);
    let reports: Vec<Vec<String>> = stdout
        .lines()
        .filter(|line| line.starts_with("over"))
        .map(|line| {
            line.split('[')
                .skip(1)
                .map(|part| part.trim_end().trim_end_matches(']').to_string())
                .collect()
        })
        .collect();
    for report in &reports {
        assert_eq!(
            report.len(),
            points.len(),
            "a report did not return every injected press:\n{stdout}"
        );
    }
    reports
}

/// One report's answers beside the points that produced them, so a failure names the
/// pixel.
fn assert_over(report: &[String], expected: &[((u32, u32), &str)]) {
    for (got, ((x, y), want)) in report.iter().zip(expected) {
        assert_eq!(got, want, "overID at ({x}, {y})");
    }
}

fn points(expected: &[((u32, u32), &str)]) -> Vec<(u32, u32)> {
    expected.iter().map(|(p, _)| *p).collect()
}

/// A closed shape is hit on its interior and on its stroke, filled or not.
///
/// * `solid`, a filled rectangle (100, 100)-(180, 160): (140, 130) inside; (99, 130),
///   centre 99.5, is half a pixel outside its left edge.
/// * `hollow`, the same size at (300, 100) with a 4 px stroke and NO fill: (340, 130)
///   is deep in the unpainted interior and is still the rectangle. Its stroke reaches
///   2 px outside the outline, to x = 298: (298, 130), centre 298.5, is on it and
///   (296, 130), centre 296.5, is 1.5 px beyond.
/// * `ghost`, a rectangle at (500, 100) with the zero paint, which draws no pixel:
///   (540, 130) is over it. That is what an invisible click target is.
/// * `disc`, a circle of radius 40 at (700, 130): (700, 130) inside; (729, 159), centre
///   41.7 px out along the diagonal, is inside its bounding box and outside the circle.
/// * `pic`, a 40x40 picture at (100, 300): (120, 320) inside, (141, 320) outside.
#[test]
fn hit_region_a_closed_shape_is_its_interior_and_its_stroke() {
    let expected = [
        ((140, 130), "solid"),
        ((99, 130), ""),
        ((340, 130), "hollow"),
        ((298, 130), "hollow"),
        ((296, 130), ""),
        ((540, 130), "ghost"),
        ((700, 130), "disc"),
        ((729, 159), ""),
        ((120, 320), "pic"),
        ((141, 320), ""),
    ];
    let reports = over(
        "canvas_hit_closed",
        r#"  MUT hollow AS canvas::Rectangle
  hollow = WITH hollow { id := "hollow", position := at(300.0, 100.0), size := canvas::Size[width := 80.0, height := 60.0], paint := canvas::stroke(color::rgb(200, 200, 200), 4.0) }
  LET hollowItem AS canvas::DrawItem = hollow
  MUT ghost AS canvas::Rectangle
  ghost = WITH ghost { id := "ghost", position := at(500.0, 100.0), size := canvas::Size[width := 80.0, height := 60.0] }
  LET ghostItem AS canvas::DrawItem = ghost
  MUT disc AS canvas::Circle
  disc = WITH disc { id := "disc", position := at(700.0, 130.0), radius := 40.0, paint := grey() }
  LET discItem AS canvas::DrawItem = disc
  RES img AS canvas::Image = canvas::createImage(1, 1, [toByte(0), toByte(255), toByte(0), toByte(255)])
  LET picItem AS canvas::DrawItem = canvas::Picture[id := "pic", shape := canvas::Rectangle[id := "", position := at(100.0, 300.0), bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 40.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := grey(), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]], clip := canvas::PictureClip.ClipOnly, fit := canvas::PictureFit.Stretch, image := img]
  canvas::present([rect("solid", 100.0, 100.0, 80.0, 60.0), hollowItem, ghostItem, discItem, picItem])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// A `Line`, an `Arc` and a `PolyLine` are hit on the stroke, and a thin one on at
/// least 3 px either side of its centre line.
///
/// * `thin`, a 1 px line along y = 300.5 from x = 100 to 300, so a press on row `r` is
///   `r - 300` px from its centre line: row 300 is on it, rows 302 and 303 are 2 and 3
///   px off and hit, row 304 is 4 px off and missed; row 297 (3 px above) hits and 296
///   misses. Past its end, (302, 300) is 2 px from the end point and hit, (305, 300) is
///   5 px and missed.
/// * `thick`, a 20 px line along y = 400.5: its own half-width, 10 px, is already more
///   than the slack, so the slack adds nothing — row 410 (10 px off) hits and row 412
///   (12 px) misses.
/// * `bow`, a 1 px arc of radius 50 about (500.5, 300.5) from 0 to a quarter turn,
///   which under a Y-down origin sweeps from +X to +Y: (536, 336), 50.2 px out at 45
///   degrees, is on it; the centre (500, 300) is 50 px inside it; and (464, 264), the
///   same distance out on the far side, is on the circle but outside the sweep.
/// * `bend`, a 1 px open path (600.5, 100.5) - (700.5, 100.5) - (700.5, 200.5):
///   (650, 102) is 2 px below its first leg, (702, 150) 2 px right of its second, and
///   (650, 150), 50 px from both, is inside the corner the two legs make and missed —
///   an open path has no interior.
#[test]
fn hit_region_an_open_shape_is_its_stroke_with_three_pixels_of_slack() {
    let expected = [
        ((200, 300), "thin"),
        ((200, 302), "thin"),
        ((200, 303), "thin"),
        ((200, 304), ""),
        ((200, 297), "thin"),
        ((200, 296), ""),
        ((302, 300), "thin"),
        ((305, 300), ""),
        ((200, 410), "thick"),
        ((200, 412), ""),
        ((536, 336), "bow"),
        ((500, 300), ""),
        ((464, 264), ""),
        ((650, 102), "bend"),
        ((702, 150), "bend"),
        ((650, 150), ""),
    ];
    let reports = over(
        "canvas_hit_open",
        r#"  LET ink AS canvas::Paint = canvas::stroke(color::rgb(200, 200, 200), 1.0)
  MUT thin AS canvas::Line
  thin = WITH thin { id := "thin", startAt := at(100.0, 300.5), endAt := at(300.0, 300.5), paint := ink }
  LET thinItem AS canvas::DrawItem = thin
  MUT thick AS canvas::Line
  thick = WITH thick { id := "thick", startAt := at(100.0, 400.5), endAt := at(300.0, 400.5), paint := canvas::stroke(color::rgb(200, 200, 200), 20.0) }
  LET thickItem AS canvas::DrawItem = thick
  MUT bow AS canvas::Arc
  bow = WITH bow { id := "bow", position := at(500.5, 300.5), radius := 50.0, startAngle := 0.0, endAngle := 1.5707963267948966, paint := ink }
  LET bowItem AS canvas::DrawItem = bow
  MUT bend AS canvas::PolyLine
  bend = WITH bend { id := "bend", points := [at(600.5, 100.5), at(700.5, 100.5), at(700.5, 200.5)], paint := ink }
  LET bendItem AS canvas::DrawItem = bend
  canvas::present([thinItem, thickItem, bowItem, bendItem])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// A `Text` is hit on its layout box — the advance width by the ascent and descent —
/// not on the ink of its glyphs, and that holds whether the run is drawn from the
/// glyph cache or, stroked, as outlines.
///
/// `AB` at `fontSize` 100 with its baseline starting at (100, 400) is the box
/// (100, 320)-(155, 420). Its ink is two 30 px squares resting on the baseline, so
/// (102, 330) is far from any ink and inside the box; (127, 419) is under the
/// baseline, in the descent; (157, 400) is 2.5 px past the advance width and (127,
/// 421) 1.5 px under the descent. `outlined` is the same string 300 px to the right
/// with a 2 px stroke, which the renderer lowers to a polygon: the same four answers.
#[test]
fn hit_region_text_is_its_layout_box() {
    let expected = [
        ((102, 330), "label"),
        ((127, 419), "label"),
        ((157, 400), ""),
        ((127, 421), ""),
        ((402, 330), "outlined"),
        ((427, 419), "outlined"),
        ((457, 400), ""),
        ((427, 421), ""),
    ];
    let reports = over(
        "canvas_hit_text",
        r#"  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("font failed")
    RETURN
  END TRAP
  LET labelItem AS canvas::DrawItem = canvas::Text[id := "label", position := at(100.0, 400.0), text := "AB", font := face, fontSize := 100.0, paint := grey(), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET outlinedItem AS canvas::DrawItem = canvas::Text[id := "outlined", position := at(400.0, 400.0), text := "AB", font := face, fontSize := 100.0, paint := canvas::stroke(color::rgb(200, 200, 200), 2.0), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([labelItem, outlinedItem])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// The shape's own clip cuts its region, and its transform is applied before the test.
///
/// * `half`, a circle of radius 50 at (200, 200) clipped to the left half of its own
///   space (the clip's ring is in the circle's coordinates, x from -60 to 0): (180,
///   200) is drawn and hit; (220, 200) is inside the circle and clipped away.
/// * `window`, a 100x100 rectangle at (400, 150) clipped to a triangle — a PATH clip —
///   with corners at its own (0, 0), (100, 0) and (0, 100): (420, 170) is inside the
///   triangle, (480, 230) inside the rectangle and beyond the hypotenuse.
/// * `leaf`, an ellipse with radii 60 and 20 at (700, 200), turned 45 degrees
///   clockwise, so its long axis runs from upper left to lower right: (740, 240), 57 px
///   along that axis, is inside; (740, 160), the same distance along the SHORT axis, is
///   inside the turned ellipse's bounding box and 37 px outside the ellipse.
/// * `rod`, a 1 px line from its own (0, 0) to (100, 0), scaled 2x and turned a
///   quarter turn at (500, 400), so it runs down from (500, 400) to (500, 600) and is 2
///   px wide. The 3 px of slack are SURFACE pixels: (502, 500), centre 2.5 px off, hits
///   and (504, 500), 4.5 px off, misses — a slack measured in the line's own units
///   would have reached 6 px.
#[test]
fn hit_region_follows_the_clip_and_the_transform() {
    let expected = [
        ((180, 200), "half"),
        ((220, 200), ""),
        ((420, 170), "window"),
        ((480, 230), ""),
        ((740, 240), "leaf"),
        ((740, 160), ""),
        ((500, 500), "rod"),
        ((502, 500), "rod"),
        ((504, 500), ""),
    ];
    let reports = over(
        "canvas_hit_clip_transform",
        r#"  MUT half AS canvas::Circle
  half = WITH half { id := "half", position := at(200.0, 200.0), radius := 50.0, paint := WITH grey() { clip := [[at(-60.0, -60.0), at(0.0, -60.0), at(0.0, 60.0), at(-60.0, 60.0)]] } }
  LET halfItem AS canvas::DrawItem = half
  MUT window AS canvas::Rectangle
  window = WITH window { id := "window", position := at(400.0, 150.0), size := canvas::Size[width := 100.0, height := 100.0], paint := WITH grey() { clip := [[at(0.0, 0.0), at(100.0, 0.0), at(0.0, 100.0)]] } }
  LET windowItem AS canvas::DrawItem = window
  MUT leaf AS canvas::Ellipse
  leaf = WITH leaf { id := "leaf", position := at(700.0, 200.0), radiusX := 60.0, radiusY := 20.0, paint := grey(), transform := canvas::Transform[a := 0.7071067811865476, b := 0.7071067811865476, c := -0.7071067811865476, d := 0.7071067811865476, tx := 0.0, ty := 0.0] }
  LET leafItem AS canvas::DrawItem = leaf
  MUT rod AS canvas::Line
  rod = WITH rod { id := "rod", position := at(500.0, 400.0), startAt := at(0.0, 0.0), endAt := at(100.0, 0.0), paint := canvas::stroke(color::rgb(200, 200, 200), 1.0), transform := canvas::Transform[a := 0.0, b := 2.0, c := -2.0, d := 0.0, tx := 0.0, ty := 0.0] }
  LET rodItem AS canvas::DrawItem = rod
  canvas::present([halfItem, windowItem, leafItem, rodItem])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// The topmost item under the point answers, whatever lies beneath it.
///
/// * `upper`, a circle of radius 40 at (200, 200), is drawn after `lower`, a rectangle
///   (100, 100)-(200, 200): (190, 190) is inside both and reports `upper`; (120, 120)
///   only the rectangle.
/// * An item with NO `id`, (420, 120)-(450, 150), lies on `under`, (400, 100)-(500,
///   200): (430, 130) reports `""` — the upper item is what the pointer is over, and it
///   has no name — while (410, 110) still reports `under`.
#[test]
fn hit_at_the_upper_of_two_overlapping_items_answers() {
    let expected = [
        ((190, 190), "upper"),
        ((120, 120), "lower"),
        ((430, 130), ""),
        ((410, 110), "under"),
    ];
    let reports = over(
        "canvas_hit_stack",
        r#"  MUT upper AS canvas::Circle
  upper = WITH upper { id := "upper", position := at(200.0, 200.0), radius := 40.0, paint := grey() }
  LET upperItem AS canvas::DrawItem = upper
  canvas::present([rect("lower", 100.0, 100.0, 100.0, 100.0), upperItem, rect("under", 400.0, 100.0, 100.0, 100.0), rect("", 420.0, 120.0, 30.0, 30.0)])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// An item inside a turned, scaled group is hit where the group puts it and missed
/// where it would have been without the group's transform.
///
/// The group holds `bar`, a rectangle at its own (50, 0)-(90, 20). The node is at
/// (450, 300) with a 2x scale and a quarter turn, which carries the group's (x, y) to
/// (450 - 2y, 300 + 2x): the rectangle lands on (410, 400)-(450, 480). (430, 440) is
/// inside that; (520, 310), inside (500, 300)-(540, 320) where a translation alone
/// would have put it, is not.
#[test]
fn hit_at_an_item_in_a_transformed_group_is_at_its_transformed_place() {
    let expected = [((430, 440), "turned::bar"), ((520, 310), "")];
    let reports = over(
        "canvas_hit_group_affine",
        r#"  canvas::setGroup("g", [rect("bar", 50.0, 0.0, 40.0, 20.0)])
  MUT turned AS canvas::Group
  turned = WITH turned { id := "turned", name := "g", position := at(450.0, 300.0), transform := canvas::Transform[a := 0.0, b := 2.0, c := -2.0, d := 0.0, tx := 0.0, ty := 0.0] }
  LET turnedItem AS canvas::DrawItem = turned
  canvas::present([turnedItem])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// An item in a layer above wins over one in a layer below, and a scene installed
/// with `presentLayers` is hit-tested at all.
///
/// `below` is (100, 100)-(200, 200) in the first layer and `above` (150, 150)-(250,
/// 250) in the second: (160, 160) is inside both, (110, 110) only the lower.
#[test]
fn hit_at_an_item_in_a_layer_above_wins() {
    let expected = [((160, 160), "above"), ((110, 110), "below")];
    let reports = over(
        "canvas_hit_layers",
        r#"  LET back AS canvas::DrawLayer = canvas::DrawLayer[items := [rect("below", 100.0, 100.0, 100.0, 100.0)]]
  LET front AS canvas::DrawLayer = canvas::DrawLayer[items := [rect("above", 150.0, 150.0, 100.0, 100.0)]]
  canvas::presentLayers([back, front])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// The id path: the `id` of every group that holds the item, outermost first, then its
/// own, joined by `::`, with an empty part left out.
///
/// The group `btn` holds `face` at its own (0, 0)-(100, 40) and an item with no `id`
/// at (0, 60)-(100, 100). `leafy` holds `leaf`, `bare` an item with no `id`, and
/// `outer` holds a node `inner` that draws `btn`.
///
/// * One group through two nodes with different `id`s: `btn` at (50, 50) as `save` and
///   at (250, 50) as `quit`. The SAME item reports `save::face` at (60, 60) and
///   `quit::face` at (260, 60).
/// * An unnamed item inside a named group reports the group's path alone: (60, 120),
///   on `save`'s second item, is `save`.
/// * A named item inside an unnamed group reports its own `id` alone: `leafy` drawn by
///   a node with no `id` at (450, 50) reports `leaf` at (460, 60).
/// * A group inside a group reports both, outermost first: `outer` drawn as `bar` at
///   (50, 300) reports `bar::inner::face` at (60, 310).
/// * Nothing named on the path reports `""`: `bare` drawn by a node with no `id` at
///   (450, 300), pressed at (460, 310).
#[test]
fn id_path_names_every_group_that_holds_the_item() {
    let expected = [
        ((60, 60), "save::face"),
        ((260, 60), "quit::face"),
        ((60, 120), "save"),
        ((460, 60), "leaf"),
        ((60, 310), "bar::inner::face"),
        ((460, 310), ""),
    ];
    let reports = over(
        "canvas_hit_id_path",
        r#"  canvas::setGroup("btn", [rect("face", 0.0, 0.0, 100.0, 40.0), rect("", 0.0, 60.0, 100.0, 40.0)])
  canvas::setGroup("leafy", [rect("leaf", 0.0, 0.0, 50.0, 50.0)])
  canvas::setGroup("bare", [rect("", 0.0, 0.0, 50.0, 50.0)])
  canvas::setGroup("outer", [node("inner", "btn", 0.0, 0.0)])
  canvas::present([node("save", "btn", 50.0, 50.0), node("quit", "btn", 250.0, 50.0), node("", "leafy", 450.0, 50.0), node("bar", "outer", 50.0, 300.0), node("", "bare", 450.0, 300.0)])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}

/// `overID` is resolved against the scene last presented, at the poll.
///
/// One press at (120, 120) and one at (320, 120), reported after each of four
/// presents:
///
/// 1. nothing presented yet: both `""`;
/// 2. `m` at (100, 100)-(150, 150): the first press is over it;
/// 3. `m` moved to (300, 100): the SAME position now reports `""` and the other `m`;
/// 4. a scene with no `id` anywhere, a rectangle under both presses: both `""`.
#[test]
fn over_id_follows_the_scene_last_presented() {
    let reports = over(
        "canvas_hit_represent",
        r#"  report("over")
  canvas::present([rect("m", 100.0, 100.0, 50.0, 50.0)])
  report("over")
  canvas::present([rect("m", 300.0, 100.0, 50.0, 50.0)])
  report("over")
  canvas::present([rect("", 100.0, 100.0, 300.0, 50.0)])
  report("over")
"#,
        &[(120, 120), (320, 120)],
    );
    assert_eq!(
        reports,
        vec![vec!["", ""], vec!["m", ""], vec!["", "m"], vec!["", ""]]
    );
}

/// A scene whose only `id` is somewhere `present` cannot see from the scene's own
/// items must still resolve `overID`: the check that lets an id-less scene skip the
/// whole test must not skip these.
///
/// 1. The only `id` is on a GROUP NODE, drawing a group of unnamed items: (110, 110)
///    reports the node's `only`.
/// 2. The only `id` is on an item INSIDE a group, drawn by a node with none — the
///    scene's own items carry no `id` at all: (110, 110) reports `deep`.
/// 3. The same node after the group is replaced by one with no `id` in it: `""`, and
///    the replacement is seen although the scene presented is byte-identical.
#[test]
fn over_id_is_resolved_when_the_only_id_is_on_or_inside_a_group() {
    let reports = over(
        "canvas_hit_group_only_id",
        r#"  canvas::setGroup("plain", [rect("", 0.0, 0.0, 50.0, 50.0)])
  canvas::present([node("only", "plain", 100.0, 100.0)])
  report("over")
  canvas::setGroup("named", [rect("deep", 0.0, 0.0, 50.0, 50.0)])
  canvas::present([node("", "named", 100.0, 100.0)])
  report("over")
  canvas::setGroup("named", [rect("", 0.0, 0.0, 50.0, 50.0)])
  canvas::present([node("", "named", 100.0, 100.0)])
  report("over")
"#,
        &[(110, 110)],
    );
    assert_eq!(reports, vec![vec!["only"], vec!["deep"], vec![""]]);
}

/// The worker's draw list is rebuilt for every changed scene it is asked about, into a
/// geometry cache that drops what the last list no longer uses and moves the rest down
/// (`canvas::geoBeginFrame`). After 300 scenes of 40 items, every one of them different
/// from the scene before, the answer must still be the last scene's.
///
/// Frame `f` puts item `k` at x = 20k + (f MOD 7), 10 px wide, on the row y = 100-110.
/// The last frame is 299, and 299 MOD 7 is 5, so `r10` covers x = 205-215: (210, 105)
/// is on it and (218, 105) is in the gap before `r11` at 225. An offset left pointing at
/// floats the cache had moved would answer some other frame's position.
#[test]
fn over_id_stays_right_across_many_changed_scenes() {
    let reports = over(
        "canvas_hit_churn",
        r#"  MUT f AS Integer = 0
  WHILE f < 300
    MUT items AS List OF canvas::DrawItem = []
    MUT k AS Integer = 0
    WHILE k < 40
      items = collections::append(items, rect("r" & toString(k), toFloat(k * 20 + (f MOD 7)), 100.0, 10.0, 10.0))
      k = k + 1
    END WHILE
    canvas::present(items)
    canvas::enableMouse(TRUE)
    LET e AS canvas::MouseEvent = canvas::pollMouse()
    canvas::enableMouse(FALSE)
    f = f + 1
  END WHILE
  report("over")
"#,
        &[(210, 105), (218, 105)],
    );
    assert_eq!(reports, vec![vec!["r10", ""]]);
}

/// The native poll builds the `MouseEvent` block by hand
/// (`func_poll_mouse.rs:lower_poll_mouse`), and `canvas::pollMouse` hands that block
/// back untouched for an idle poll and for an event over nothing. So the hand-built
/// block has to be a record the rest of the language can copy and read: its two
/// inlined fields — `position`, and `overID`'s empty string — sized and placed as the
/// compiler would have placed them.
///
/// Four polls go into a list, which copies each record by its own computed size, and
/// every field is read back out of the copies: a Shift-press over `box`, a plain press
/// over nothing, and two idle polls. A block sized short would lose `position` or
/// `overID` in the copy; one with `overID`'s slot left 0 would read as a record with no
/// string at all.
#[test]
fn a_polled_event_is_a_whole_record() {
    let stdout = run(
        "canvas_hit_whole_record",
        r#"  canvas::present([rect("box", 100.0, 100.0, 50.0, 50.0)])
  canvas::enableMouse(TRUE)
  MUT seen AS List OF canvas::MouseEvent = []
  MUT n AS Integer = 0
  WHILE n < 4
    seen = collections::append(seen, canvas::pollMouse())
    n = n + 1
  END WHILE
  canvas::enableMouse(FALSE)
  FOR EACH e IN seen
    MUT kind AS String = "other"
    IF e.kind = canvas::MouseKind.None THEN
      kind = "idle"
    END IF
    IF e.kind = canvas::MouseKind.Down THEN
      kind = "down"
    END IF
    MUT button AS String = "none"
    IF e.button = canvas::MouseButton.Left THEN
      button = "left"
    END IF
    MUT mods AS String = ""
    IF e.shift THEN
      mods = mods & "S"
    END IF
    IF e.ctrl THEN
      mods = mods & "C"
    END IF
    IF e.alt THEN
      mods = mods & "A"
    END IF
    io::print("whole " & kind & " " & button & " " & toString(e.position.x) & "," & toString(e.position.y) & " mods=" & mods & " over=[" & e.overID & "]")
  NEXT
"#,
        &format!("{}{}", press(4, 120, 130), press(0, 300, 310)),
    );
    let lines: Vec<&str> = stdout.lines().filter(|l| l.starts_with("whole")).collect();
    assert_eq!(
        lines,
        vec![
            "whole down left 120.00,130.00 mods=S over=[box]",
            "whole down left 300.00,310.00 mods= over=[]",
            "whole idle none 0.00,0.00 mods= over=[]",
            "whole idle none 0.00,0.00 mods= over=[]",
        ],
        "{stdout}"
    );
}

/// The Rust-side text of a `canvas::Picture` with `Stretch` and `ClipOnly`.
fn picture(id: &str, shape: &str, image: &str) -> String {
    format!(
        "canvas::Picture[id := \"{id}\", shape := {shape}, clip := canvas::PictureClip.ClipOnly, \
         fit := canvas::PictureFit.Stretch, image := {image}]"
    )
}

/// plan-172-L: a `Picture` is hit as its shape, and its own `id` goes in front of the
/// shape's.
///
/// * `a` and `b` are two pictures built on ONE shape value, the circle `frame` of
///   radius 50 at (200, 200). They are two entries with two paths: `b` is presented
///   last, so (200, 200) is `b::frame` — and `a::frame` once the scene is presented
///   again with the two the other way round.
/// * `avatar` is a named picture on an UNNAMED rounded rectangle, (400, 150) 80 by 80
///   with 20 px corners: (440, 190) reports the picture's `id` alone. (401, 151),
///   centre half a pixel inside the rectangle's box, is outside its rounded corner —
///   the picture is hit as its shape, not as its box.
/// * An unnamed picture on the circle `ring`, radius 40 at (600, 200), reports the
///   shape's `id` alone at (600, 200); (630, 230), centre 43.1 px out, is outside it.
/// * `bar` is a picture on a `Line`, which has no inside: it draws nothing, and
///   (200, 400), on the line, is over nothing.
/// * The group `cardg` holds a picture `avatar` on a circle `frame` of radius 30 about
///   its origin; the node `card` puts it at (700, 400), and (700, 400) reports group,
///   picture, shape in that order.
#[test]
fn id_path_a_picture_goes_in_front_of_its_shape() {
    let expected = [
        ((200, 200), "b::frame"),
        ((440, 190), "avatar"),
        ((401, 151), ""),
        ((600, 200), "ring"),
        ((630, 230), ""),
        ((200, 400), ""),
        ((700, 400), "card::avatar::frame"),
    ];
    let swapped = [
        ((200, 200), "a::frame"),
        ((440, 190), ""),
        ((401, 151), ""),
        ((600, 200), ""),
        ((630, 230), ""),
        ((200, 400), ""),
        ((700, 400), ""),
    ];
    let body = format!(
        r#"  RES img AS canvas::Image = canvas::createImage(1, 1, [toByte(0), toByte(255), toByte(0), toByte(255)])
  RES inCard AS canvas::Image = canvas::createImage(1, 1, [toByte(255), toByte(0), toByte(0), toByte(255)])
  MUT frame AS canvas::Circle
  frame = WITH frame {{ id := "frame", position := at(200.0, 200.0), radius := 50.0, paint := grey() }}
  LET a AS canvas::DrawItem = {a}
  LET b AS canvas::DrawItem = {b}
  MUT soft AS canvas::Rectangle
  soft = WITH soft {{ position := at(400.0, 150.0), size := canvas::Size[width := 80.0, height := 80.0], radiusTL := 20.0, radiusTR := 20.0, radiusBL := 20.0, radiusBR := 20.0, paint := grey() }}
  LET avatar AS canvas::DrawItem = {avatar}
  MUT ring AS canvas::Circle
  ring = WITH ring {{ id := "ring", position := at(600.0, 200.0), radius := 40.0, paint := grey() }}
  LET ringed AS canvas::DrawItem = {ringed}
  MUT stroke AS canvas::Line
  stroke = WITH stroke {{ id := "stroke", startAt := at(100.0, 400.5), endAt := at(300.0, 400.5), paint := canvas::stroke(color::rgb(200, 200, 200), 20.0) }}
  LET bar AS canvas::DrawItem = {bar}
  MUT inner AS canvas::Circle
  inner = WITH inner {{ id := "frame", radius := 30.0, paint := grey() }}
  canvas::setGroup("cardg", [{carded}])
  canvas::present([a, b, avatar, ringed, bar, node("card", "cardg", 700.0, 400.0)])
  report("over")
  canvas::present([b, a])
  report("over")
"#,
        a = picture("a", "frame", "img"),
        b = picture("b", "frame", "img"),
        avatar = picture("avatar", "soft", "img"),
        ringed = picture("", "ring", "img"),
        bar = picture("bar", "stroke", "img"),
        carded = picture("avatar", "inner", "inCard"),
    );
    let reports = over("canvas_hit_picture", &body, &points(&expected));
    assert_over(&reports[0], &expected);
    assert_over(&reports[1], &swapped);
}

/// plan-150-A: a particle is never under the mouse. A `canvas::ParticleSystem` drawn
/// OVER a named rectangle leaves `overID` naming the rectangle, and one drawn over
/// nothing reports nothing — even when its template has an `id` of its own.
///
/// * `under`, a rectangle (100, 100)-(300, 200), presented FIRST.
/// * a system of one particle, a disc of radius 60 named `spark` at (200, 150),
///   presented after it and so drawn on top: (200, 150) is the centre of both, and the
///   answer is the rectangle the disc covers.
/// * a second such system at (600, 400), over the bare surface: (600, 400) is on the
///   disc, and nothing is reported.
#[test]
fn hit_region_a_particle_system_is_never_hit() {
    let expected = [((200, 150), "under"), ((600, 400), ""), ((50, 50), "")];
    let reports = over(
        "canvas_hit_particles",
        r#"  MUT dot AS canvas::Circle
  LET spark AS canvas::AutoShape = WITH dot { id := "spark", radius := 60.0, paint := grey() }
  MUT origin AS canvas::Circle
  LET emitter AS canvas::Shape = origin
  MUT motion AS canvas::Motion
  MUT emission AS canvas::Emission
  LET white AS color::Color = color::rgb(255, 255, 255)
  LET look AS canvas::Appearance = canvas::Appearance[startColor := white, endColor := white, startScale := 1.0, endScale := 1.0, scaleVar := 0.0]
  LET once AS canvas::Emission = WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 1, burst := 1, lifetime := 10.0 }
  LET over AS canvas::DrawItem = canvas::ParticleSystem[position := at(200.0, 150.0), item := spark, emitter := emitter, emission := once, motion := motion, appearance := look, seed := 1, time := 1.0]
  LET alone AS canvas::DrawItem = canvas::ParticleSystem[position := at(600.0, 400.0), item := spark, emitter := emitter, emission := once, motion := motion, appearance := look, seed := 1, time := 1.0]
  canvas::present([rect("under", 100.0, 100.0, 200.0, 100.0), over, alone])
  report("over")
"#,
        &points(&expected),
    );
    assert_over(&reports[0], &expected);
}
