//! The arithmetic of a `canvas::Picture` drawn inside a shape (plan-172-L Phase 1):
//! where the image goes under each `PictureFit`, the flattened outline of a circle, and
//! the intersection of two clips.
//!
//! These are pure MFBASIC functions
//! (`src/codegen/builtins/canvas/helper_picture.rs:PICTURE_GEOMETRY`). The compiler is a
//! binary crate, so a test cannot link that text; this file reads it out of the source
//! it is declared in — with the two small chunks of `helper_shapes.rs` it calls, the
//! float helpers and the deterministic sine and cosine — compiles it behind a driver,
//! and runs it. What is run is the text the compiler injects, with only the `__canvas_`
//! prefix, which user source may not spell, renamed.
//!
//! The driver prints one line per case, `name key=value …`, with an area or a distance
//! as a whole number of millionths (`toString` of a `Float` keeps six significant
//! digits, which is not enough to tell a pentagon from a rounding). An intersection is
//! measured three ways: its AREA, by the shoelace sum over its edges (they are oriented,
//! so the sum is the area); its EDGE count; and point membership at fixed probes, by
//! the even-odd crossing count the renderer's clip uses. `seams` counts edges that are
//! not boundary — an edge whose two sides, a quarter pixel either way, are both inside
//! or both outside.

#[path = "../common/mod.rs"]
mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

/// The text of raw-string constant `name` in `file` under `src/codegen/builtins/canvas`.
fn chunk(file: &str, name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/codegen/builtins/canvas")
        .join(file);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {file}: {e}"));
    let opens = format!("const {name}: &str =\nr#\"");
    let start = text
        .find(&opens)
        .unwrap_or_else(|| panic!("{file} declares no raw string `{name}`"))
        + opens.len();
    let end = start + text[start..].find("\"#;").expect("the raw string closes");
    text[start..end]
        .replace("__canvas_", "k_")
        .replace("__CANVAS_", "K_")
}

const DRIVER: &str = r#"
FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC square(x0 AS Float, y0 AS Float, x1 AS Float, y1 AS Float) AS List OF vector::Float2
  RETURN [at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1)]
END FUNC

' A five-point star as ONE self-crossing ring: every second vertex of a pentagon of
' circumradius `r`.
FUNC star(r AS Float) AS List OF vector::Float2
  MUT ring AS List OF vector::Float2 = []
  MUT k AS Integer = 0
  WHILE k < 5
    LET a AS Float = 6.283185307179586 * toFloat((k * 2) MOD 5) / 5.0 - 1.5707963267948966
    ring = collections::append(ring, at(r * math::cos(a), r * math::sin(a)))
    k = k + 1
  END WHILE
  RETURN ring
END FUNC

' Even-odd membership by crossing count, as the renderer's clip does it.
FUNC inside(e AS List OF Float, x AS Float, y AS Float) AS Boolean
  MUT odd AS Boolean = FALSE
  MUT i AS Integer = 0
  WHILE i + 4 < len(e)
    LET x0 AS Float = collections::getOr(e, i, 0.0)
    LET y0 AS Float = collections::getOr(e, i + 1, 0.0)
    LET dx AS Float = collections::getOr(e, i + 2, 0.0)
    LET dy AS Float = collections::getOr(e, i + 3, 0.0)
    IF (y0 > y) <> (y0 + dy > y) THEN
      IF x0 + (y - y0) * dx / dy > x THEN
        odd = NOT odd
      END IF
    END IF
    i = i + 5
  END WHILE
  RETURN odd
END FUNC

FUNC flag(b AS Boolean) AS String
  IF b THEN
    RETURN "1"
  END IF
  RETURN "0"
END FUNC

' One intersection: its area, its edge count, its seams, and three probes.
FUNC report(name AS String, e AS List OF Float, p1 AS vector::Float2, p2 AS vector::Float2, p3 AS vector::Float2) AS Nothing
  MUT area AS Float = 0.0
  MUT seams AS Integer = 0
  MUT i AS Integer = 0
  WHILE i + 4 < len(e)
    LET x0 AS Float = collections::getOr(e, i, 0.0)
    LET y0 AS Float = collections::getOr(e, i + 1, 0.0)
    LET dx AS Float = collections::getOr(e, i + 2, 0.0)
    LET dy AS Float = collections::getOr(e, i + 3, 0.0)
    area = area + (x0 * (y0 + dy) - (x0 + dx) * y0) / 2.0
    LET length AS Float = math::sqrt(dx * dx + dy * dy)
    LET mx AS Float = x0 + dx / 2.0
    LET my AS Float = y0 + dy / 2.0
    LET nx AS Float = 0.25 * dy / length
    LET ny AS Float = 0.0 - 0.25 * dx / length
    IF inside(e, mx + nx, my + ny) = inside(e, mx - nx, my - ny) THEN
      seams = seams + 1
    END IF
    i = i + 5
  END WHILE
  IF area < 0.0 THEN
    area = 0.0 - area
  END IF
  io::print(name & " areaMicro=" & toString(toInt(area * 1000000.0 + 0.5)) & " edges=" & toString(len(e) / 5) & " seams=" & toString(seams) & " p1=" & flag(inside(e, p1.x, p1.y)) & " p2=" & flag(inside(e, p2.x, p2.y)) & " p3=" & flag(inside(e, p3.x, p3.y)))
END FUNC

FUNC fit(name AS String, x1 AS Float, y1 AS Float, iw AS Float, ih AS Float, mode AS Integer) AS Nothing
  LET r AS List OF Float = k_pictureFit(0.0, 0.0, x1, y1, iw, ih, mode)
  io::print(name & " x=" & toString(collections::getOr(r, 0, 0.0)) & " y=" & toString(collections::getOr(r, 1, 0.0)) & " w=" & toString(collections::getOr(r, 2, 0.0)) & " h=" & toString(collections::getOr(r, 3, 0.0)))
END FUNC

FUNC main() AS Nothing
  ' --- fit: a 200x100 image in a 100x200 box, and a 100x200 image in a 200x100 box ---
  fit("wide-stretch", 100.0, 200.0, 200.0, 100.0, 0)
  fit("wide-contain", 100.0, 200.0, 200.0, 100.0, 1)
  fit("wide-cover", 100.0, 200.0, 200.0, 100.0, 2)
  fit("tall-stretch", 200.0, 100.0, 100.0, 200.0, 0)
  fit("tall-contain", 200.0, 100.0, 100.0, 200.0, 1)
  fit("tall-cover", 200.0, 100.0, 100.0, 200.0, 2)

  ' --- outline: a circle of radius 100 about (30, 40) ---
  LET ring AS List OF vector::Float2 = k_ellipseRing(30.0, 40.0, 100.0, 100.0)
  MUT vertexOff AS Float = 0.0
  MUT chordOff AS Float = 0.0
  MUT i AS Integer = 0
  WHILE i < len(ring)
    LET a AS vector::Float2 = collections::getOr(ring, i, at(0.0, 0.0))
    LET b AS vector::Float2 = collections::getOr(ring, (i + 1) MOD len(ring), at(0.0, 0.0))
    vertexOff = k_maxF(vertexOff, k_absF(math::sqrt((a.x - 30.0) * (a.x - 30.0) + (a.y - 40.0) * (a.y - 40.0)) - 100.0))
    LET mx AS Float = (a.x + b.x) / 2.0 - 30.0
    LET my AS Float = (a.y + b.y) / 2.0 - 40.0
    chordOff = k_maxF(chordOff, 100.0 - math::sqrt(mx * mx + my * my))
    i = i + 1
  END WHILE
  io::print("circle points=" & toString(len(ring)) & " vertexNano=" & toString(toInt(vertexOff * 1000000000.0 + 0.5)) & " chordMicro=" & toString(toInt(chordOff * 1000000.0 + 0.5)))
  io::print("flat points=" & toString(len(k_ellipseRing(0.0, 0.0, 0.0, 5.0))))

  ' --- intersections ---
  report("squares", k_clipIntersect([square(0.0, 0.0, 10.0, 10.0)], FALSE, [square(5.0, 5.0, 15.0, 15.0)], FALSE), at(7.0, 7.0), at(3.0, 3.0), at(12.0, 12.0))
  report("disjoint", k_clipIntersect([square(0.0, 0.0, 10.0, 10.0)], FALSE, [square(20.0, 20.0, 30.0, 30.0)], FALSE), at(5.0, 5.0), at(25.0, 25.0), at(15.0, 15.0))
  report("square-in-circle", k_clipIntersect([k_ellipseRing(0.0, 0.0, 10.0, 10.0)], FALSE, [square(0.0 - 5.0, 0.0 - 5.0, 5.0, 5.0)], FALSE), at(0.0, 0.0), at(4.5, 4.5), at(7.0, 0.0))
  LET frame AS List OF vector::Float2 = square(0.0 - 20.0, 0.0 - 20.0, 20.0, 20.0)
  report("star-evenodd", k_clipIntersect([star(10.0)], FALSE, [frame], FALSE), at(0.0, 0.0), at(0.0, 0.0 - 8.0), at(15.0, 15.0))
  report("star-nonzero", k_clipIntersect([star(10.0)], TRUE, [frame], FALSE), at(0.0, 0.0), at(0.0, 0.0 - 8.0), at(15.0, 15.0))
  report("holed", k_clipIntersect([square(0.0, 0.0, 20.0, 20.0), square(5.0, 5.0, 15.0, 15.0)], FALSE, [square(0.0 - 5.0, 8.0, 25.0, 12.0)], FALSE), at(2.0, 10.0), at(10.0, 10.0), at(17.0, 10.0))
  ' A circle cut by the half-plane x >= 0: one straight side and half the chords.
  LET disc AS List OF vector::Float2 = k_ellipseRing(0.0, 0.0, 10.0, 10.0)
  io::print("disc points=" & toString(len(disc)))
  report("half-disc", k_clipIntersect([disc], FALSE, [square(0.0, 0.0 - 20.0, 20.0, 20.0)], FALSE), at(5.0, 0.0), at(0.0 - 5.0, 0.0), at(9.0, 9.0))
END FUNC
"#;

/// The driver's output, one entry per case: `name → { key → value }`.
fn cases() -> &'static HashMap<String, HashMap<String, f64>> {
    static RUN: OnceLock<HashMap<String, HashMap<String, f64>>> = OnceLock::new();
    RUN.get_or_init(|| {
        let source = format!(
            "IMPORT collections\nIMPORT io\nIMPORT math\nIMPORT vector\n\n{}\n\n{}\n\n{}\n{DRIVER}",
            chunk("helper_shapes.rs", "FLOAT_UTIL"),
            chunk("helper_shapes.rs", "TRIG"),
            chunk("helper_picture.rs", "PICTURE_GEOMETRY"),
        );
        let project = common::temp_project("canvas_picture_fit", &source);
        let binary = common::build_project(&project);
        let output = Command::new(&binary)
            .output()
            .expect("run the picture-geometry driver");
        assert!(
            output.status.success(),
            "the driver {}:\n{}",
            common::exit_description(&output.status),
            String::from_utf8_lossy(&output.stderr),
        );
        let _ = std::fs::remove_dir_all(&project);
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let name = parts.next()?.to_string();
                let values = parts
                    .filter_map(|part| {
                        let (key, value) = part.split_once('=')?;
                        Some((key.to_string(), value.parse().ok()?))
                    })
                    .collect();
                Some((name, values))
            })
            .collect()
    })
}

fn value(case: &str, key: &str) -> f64 {
    *cases()
        .get(case)
        .unwrap_or_else(|| panic!("the driver printed no `{case}` line"))
        .get(key)
        .unwrap_or_else(|| panic!("`{case}` has no `{key}`"))
}

/// An intersection's area, from the millionths the driver printed.
fn area(case: &str) -> f64 {
    value(case, "areaMicro") / 1e6
}

fn assert_area(case: &str, want: f64) {
    let got = area(case);
    assert!((got - want).abs() < 2e-6, "{case}: area {got}, want {want}");
}

fn assert_near(case: &str, key: &str, want: f64) {
    let got = value(case, key);
    assert!(
        (got - want).abs() < 1e-6,
        "{case} {key}: got {got}, want {want}"
    );
}

fn assert_rect(case: &str, x: f64, y: f64, w: f64, h: f64) {
    for (key, want) in [("x", x), ("y", y), ("w", w), ("h", h)] {
        assert_near(case, key, want);
    }
}

/// `Stretch` is the box, whatever the image's proportions.
#[test]
fn canvas_picture_fit_stretch_fills_the_box() {
    assert_rect("wide-stretch", 0.0, 0.0, 100.0, 200.0);
    assert_rect("tall-stretch", 0.0, 0.0, 200.0, 100.0);
}

/// `Contain` is the largest rectangle of the image's proportions inside the box,
/// centred: a 2:1 image in a 100 by 200 box is 100 by 50, 75 down.
#[test]
fn canvas_picture_fit_contain_fits_inside_and_centres() {
    assert_rect("wide-contain", 0.0, 75.0, 100.0, 50.0);
    assert_rect("tall-contain", 75.0, 0.0, 50.0, 100.0);
}

/// `Cover` is the smallest rectangle of the image's proportions that covers the box,
/// centred: the same image is 400 by 200, 150 to the left.
#[test]
fn canvas_picture_fit_cover_covers_and_centres() {
    assert_rect("wide-cover", -150.0, 0.0, 400.0, 200.0);
    assert_rect("tall-cover", 0.0, -150.0, 200.0, 400.0);
}

/// The outline of a circle: every vertex is on the circle, and no chord is more than
/// 0.05 pixels inside it. A shape with no extent has no outline.
///
/// "On the circle" is to a ten-thousandth of a pixel, not to the last bit: the ring is
/// built with the canvas's own sine and cosine (`helper_shapes.rs:TRIG`), which are
/// deterministic across platforms and renderers and good to `4.6e-7` — `4.6e-5` of a
/// pixel at this radius. The first cut of this test asked for a nanometre and failed.
#[test]
fn canvas_picture_fit_a_circle_outline_is_within_a_twentieth_of_a_pixel() {
    assert!(value("circle", "vertexNano") <= 100_000.0);
    let chord = value("circle", "chordMicro") / 1e6;
    assert!(chord > 0.0 && chord <= 0.05, "chordOff {chord}");
    assert!(value("circle", "points") >= 16.0);
    assert_eq!(value("flat", "points"), 0.0);
}

/// Two overlapping squares give the overlap square; two disjoint ones give no edges.
#[test]
fn canvas_picture_fit_two_squares_intersect_in_their_overlap() {
    assert_area("squares", 25.0);
    assert_eq!(value("squares", "edges"), 4.0);
    assert_eq!(value("squares", "seams"), 0.0);
    assert_eq!(
        (
            value("squares", "p1"),
            value("squares", "p2"),
            value("squares", "p3")
        ),
        (1.0, 0.0, 0.0)
    );
    assert_eq!(value("disjoint", "edges"), 0.0);
    assert_area("disjoint", 0.0);
}

/// A square inside a circle's outline is the square: four edges, and none of the
/// circle's.
#[test]
fn canvas_picture_fit_a_square_inside_a_circle_is_the_square() {
    assert_area("square-in-circle", 100.0);
    assert_eq!(value("square-in-circle", "edges"), 4.0);
    assert_eq!(value("square-in-circle", "seams"), 0.0);
    assert_eq!(
        (
            value("square-in-circle", "p1"),
            value("square-in-circle", "p2"),
            value("square-in-circle", "p3")
        ),
        (1.0, 1.0, 0.0)
    );
}

/// Each set keeps its own fill rule. A five-point star drawn as one self-crossing ring
/// has a pentagon in the middle that the even-odd rule leaves out and the non-zero rule
/// fills, so the two intersections differ by exactly that pentagon: circumradius
/// `10 * cos 72° / cos 36°`, area `5/2 * r² * sin 72°`.
#[test]
fn canvas_picture_fit_each_ring_set_keeps_its_own_fill_rule() {
    let r = 10.0 * (72f64.to_radians().cos() / 36f64.to_radians().cos());
    let pentagon = 2.5 * r * r * 72f64.to_radians().sin();
    let differ = area("star-nonzero") - area("star-evenodd");
    assert!(
        (differ - pentagon).abs() < 4e-6,
        "the two rules differ by {differ}, the pentagon is {pentagon}"
    );
    // The centre is in the pentagon; a point of the star's top arm is in both.
    assert_eq!(value("star-evenodd", "p1"), 0.0);
    assert_eq!(value("star-nonzero", "p1"), 1.0);
    assert_eq!(value("star-evenodd", "p2"), 1.0);
    assert_eq!(value("star-nonzero", "p2"), 1.0);
    assert_eq!(value("star-evenodd", "seams"), 0.0);
    assert_eq!(value("star-nonzero", "seams"), 0.0);
}

/// A square with a square hole, against a bar that crosses the hole, is two pieces.
#[test]
fn canvas_picture_fit_a_bar_across_a_hole_is_two_pieces() {
    assert_area("holed", 40.0);
    assert_eq!(value("holed", "edges"), 8.0);
    assert_eq!(value("holed", "seams"), 0.0);
    assert_eq!(
        (
            value("holed", "p1"),
            value("holed", "p2"),
            value("holed", "p3")
        ),
        (1.0, 0.0, 1.0)
    );
}

/// No returned edge has the region on both sides or on neither, and a source edge that
/// bounds the region across several bands comes back as ONE edge. A circle cut by a
/// half-plane has a band boundary at every vertex of the half that is cut away, so
/// every surviving chord is crossed by one — and the result is still about half the
/// circle's chords and one straight side, not a piece per band.
#[test]
fn canvas_picture_fit_no_edge_is_interior_and_runs_are_joined() {
    assert_eq!(value("half-disc", "seams"), 0.0);
    let points = value("disc", "points");
    let edges = value("half-disc", "edges");
    assert!(
        edges <= points / 2.0 + 4.0,
        "{edges} edges for half of a {points}-point circle"
    );
    assert!(edges >= points / 2.0 - 2.0);
    // Half the polygon's area: n/2 * r² * sin(2π/n), halved.
    let whole = points / 2.0 * 100.0 * (2.0 * std::f64::consts::PI / points).sin();
    // To a thousandth: the vertices are the canvas's deterministic sine and cosine's,
    // not `f64::sin`'s (see the circle-outline test).
    let half = area("half-disc");
    assert!(
        (half - whole / 2.0).abs() < 1e-3,
        "half-disc: area {half}, want {}",
        whole / 2.0
    );
    assert_eq!(
        (
            value("half-disc", "p1"),
            value("half-disc", "p2"),
            value("half-disc", "p3")
        ),
        (1.0, 0.0, 0.0)
    );
}
