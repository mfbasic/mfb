//! The native geometry builder (`canvas::geoBuild`) — bug-686.
//!
//! `canvas::geoBuild` replaces the MFBASIC header builders for the common kinds, and its
//! record must be **bit-identical** to theirs: the software rasteriser reads it, and its
//! goldens are exact. A `--debug` build run with `MFB_CANVAS_GEO_VERIFY=1` rebuilds every
//! natively built record with the MFBASIC builders and counts the ones that differ in any
//! bit. The stats line reports `geoNative=` (records the native builder produced),
//! `geoVerified=` (records compared) and `geoVerifyMismatches=`.

#[path = "../common/mod.rs"]
mod common;

use std::process::Command;

/// Build a `--debug --app` program, run it headless, synchronously and with the geometry
/// check on, and return one `MFB_CANVAS_STATS` line per frame.
fn stats(name: &str, source: &str) -> Vec<String> {
    let project = common::temp_project(name, source);
    let binary = common::build_app_debug(&project, name);
    let stats = project.join("stats.txt");
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_GEO_VERIFY", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    lines
}

/// One `name=value` field of a stats line, as a number.
fn field(line: &str, name: &str) -> i64 {
    line.split_whitespace()
        .find_map(|f| f.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no `{name}=` field in {line:?}"))
        .parse()
        .unwrap_or_else(|e| panic!("`{name}` is not a number in {line:?}: {e}"))
}

/// Every paint the matrix crosses with every shape: fill and stroke each opaque-ish or
/// fully transparent, four stroke widths (none, thin, fractional, negative), the four
/// blend modes, and no clip / a fractional clip / a negative-width clip. A second group
/// keeps a one-stop gradient (not a gradient, so still native, but its kind and points
/// are still written) and an all-negative-zero transform (the identity by float `=`).
const PAINTS: &str = r#"
FUNC paints() AS List OF canvas::Paint
  MUT out AS List OF canvas::Paint = []
  LET fills AS List OF color::Color = [color::rgba(10, 20, 30, 200), color::rgba(0, 0, 0, 0)]
  LET strokes AS List OF color::Color = [color::rgba(200, 100, 50, 255), color::rgba(1, 2, 3, 0)]
  LET widths AS List OF Float = [0.0, 1.0, 2.75, 0.0 - 1.5]
  LET modes AS List OF canvas::BlendMode = [canvas::BlendMode.Normal, canvas::BlendMode.Multiply, canvas::BlendMode.Screen, canvas::BlendMode.Add]
  LET clips AS List OF canvas::Bounds = [canvas::Bounds[x := 0.0, y := 0.0, w := 0.0, h := 0.0], canvas::Bounds[x := 5.5, y := 0.0 - 3.25, w := 100.125, h := 50.0], canvas::Bounds[x := 10.0, y := 10.0, w := 0.0 - 5.0, h := 3.0]]
  FOR EACH f IN fills
    FOR EACH s IN strokes
      FOR EACH w IN widths
        FOR EACH m IN modes
          FOR EACH c IN clips
            out = collections::append(out, WITH canvas::fill(f) { stroke := s, strokeWidth := w, blend := m, clip := c })
          NEXT
        NEXT
      NEXT
    NEXT
  NEXT
  LET nz AS Float = 0.0 * (0.0 - 1.0)
  LET one AS canvas::GradientStop = canvas::GradientStop[offset := 0.25, color := color::rgb(9, 8, 7)]
  LET ramp AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Radial, startPoint := canvas::Point[x := 1.5, y := 2.5], endPoint := canvas::Point[x := 0.0 - 3.5, y := 4.25], stops := [one]]
  LET flat AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 7.0, y := 8.0], endPoint := canvas::Point[x := 9.0, y := 10.0], stops := []]
  LET negZero AS canvas::Transform = canvas::Transform[a := nz, b := nz, c := nz, d := nz, tx := nz, ty := nz]
  out = collections::append(out, WITH canvas::fill(color::rgb(1, 2, 3)) { stroke := color::rgb(4, 5, 6), strokeWidth := 2.0, fillGradient := ramp })
  out = collections::append(out, WITH canvas::fill(color::rgb(1, 2, 3)) { fillGradient := flat, blend := canvas::BlendMode.Add })
  out = collections::append(out, WITH canvas::stroke(color::rgb(4, 5, 6), 3.0) { transform := negZero })
  RETURN out
END FUNC
"#;

/// Every native kind at fractional, negative, large and tiny coordinates, and every
/// degenerate size the builders special-case: zero and negative width, height and
/// radius, a corner radius below zero and past the limit, zero-length and
/// negative-zero lines, and polygons of 0, 1 and 2 points, collinear and with a
/// duplicate vertex.
const SHAPES: &str = r#"
FUNC pt(x AS Float, y AS Float) AS canvas::Point
  RETURN canvas::Point[x := x, y := y]
END FUNC

FUNC shapes(p AS canvas::Paint) AS List OF canvas::DrawItem
  MUT out AS List OF canvas::DrawItem = []
  LET nz AS Float = 0.0 * (0.0 - 1.0)
  LET r1 AS canvas::DrawItem = canvas::Rectangle[x := 10.5, y := 20.25, w := 30.0, h := 40.0, paint := p]
  LET r2 AS canvas::DrawItem = canvas::Rectangle[x := 0.0 - 15.75, y := 0.0 - 3.5, w := 7.25, h := 9.125, paint := p]
  LET r3 AS canvas::DrawItem = canvas::Rectangle[x := 1000000.3, y := 0.0 - 2000000.0, w := 1234.5, h := 0.001, paint := p]
  LET r4 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 0.0, h := 10.0, paint := p]
  LET r5 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 0.0 - 3.0, h := 10.0, paint := p]
  LET r6 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 10.0, h := 0.0, paint := p]
  LET r7 AS canvas::DrawItem = canvas::Rectangle[x := 5.0, y := 5.0, w := 10.0, h := 0.0 - 0.5, paint := p]
  LET r8 AS canvas::DrawItem = canvas::Rectangle[x := nz, y := nz, w := 0.000000001, h := 0.3, paint := p]
  LET q1 AS canvas::DrawItem = canvas::RoundedRect[x := 10.0, y := 10.0, w := 40.0, h := 20.0, cornerRadius := 5.0, paint := p]
  LET q2 AS canvas::DrawItem = canvas::RoundedRect[x := 10.0, y := 10.0, w := 40.0, h := 20.0, cornerRadius := 100.0, paint := p]
  LET q3 AS canvas::DrawItem = canvas::RoundedRect[x := 10.0, y := 10.0, w := 40.0, h := 20.0, cornerRadius := 0.0 - 3.0, paint := p]
  LET q4 AS canvas::DrawItem = canvas::RoundedRect[x := 0.0 - 3.3, y := 7.7, w := 0.5, h := 8.25, cornerRadius := 0.2, paint := p]
  LET q5 AS canvas::DrawItem = canvas::RoundedRect[x := 1.0, y := 1.0, w := 0.0, h := 8.0, cornerRadius := 2.0, paint := p]
  LET c1 AS canvas::DrawItem = canvas::Circle[x := 100.5, y := 50.25, radius := 20.0, paint := p]
  LET c2 AS canvas::DrawItem = canvas::Circle[x := 0.0 - 7.0, y := 0.0 - 8.0, radius := 0.5, paint := p]
  LET c3 AS canvas::DrawItem = canvas::Circle[x := 300000.0, y := 1.5, radius := 12345.678, paint := p]
  LET c4 AS canvas::DrawItem = canvas::Circle[x := 3.0, y := 3.0, radius := 0.0, paint := p]
  LET c5 AS canvas::DrawItem = canvas::Circle[x := 3.0, y := 3.0, radius := 0.0 - 1.0, paint := p]
  LET l1 AS canvas::DrawItem = canvas::Line[x1 := 1.0, y1 := 2.0, x2 := 300.0, y2 := 200.0, cap := canvas::CapStyle.Round, paint := p]
  LET l2 AS canvas::DrawItem = canvas::Line[x1 := 1.0, y1 := 2.0, x2 := 300.0, y2 := 200.0, cap := canvas::CapStyle.Butt, paint := p]
  LET l3 AS canvas::DrawItem = canvas::Line[x1 := 5.0, y1 := 5.0, x2 := 5.0, y2 := 5.0, cap := canvas::CapStyle.Round, paint := p]
  LET l4 AS canvas::DrawItem = canvas::Line[x1 := 0.0 - 10.5, y1 := 3.25, x2 := 7.75, y2 := 0.0 - 2.0, cap := canvas::CapStyle.Butt, paint := p]
  LET l5 AS canvas::DrawItem = canvas::Line[x1 := 1000000.0, y1 := 1000000.0, x2 := 1000001.0, y2 := 999999.0, cap := canvas::CapStyle.Round, paint := p]
  LET l6 AS canvas::DrawItem = canvas::Line[x1 := nz, y1 := 0.0, x2 := 0.0, y2 := nz, cap := canvas::CapStyle.Butt, paint := p]
  LET none AS List OF canvas::Point = []
  LET g1 AS canvas::DrawItem = canvas::Polygon[points := [pt(10.0, 10.0), pt(60.0, 15.5), pt(30.25, 70.0)], paint := p]
  LET g2 AS canvas::DrawItem = canvas::Polygon[points := [pt(1.0, 2.0), pt(3.0, 4.0)], paint := p]
  LET g3 AS canvas::DrawItem = canvas::Polygon[points := [pt(1.0, 2.0)], paint := p]
  LET g4 AS canvas::DrawItem = canvas::Polygon[points := none, paint := p]
  LET g5 AS canvas::DrawItem = canvas::Polygon[points := [pt(0.0, 0.0), pt(5.0, 5.0), pt(10.0, 10.0)], paint := p]
  LET g6 AS canvas::DrawItem = canvas::Polygon[points := [pt(0.0, 0.0), pt(8.0, 0.0), pt(8.0, 0.0), pt(8.0, 8.0), pt(0.0, 8.0)], paint := p]
  LET g7 AS canvas::DrawItem = canvas::Polygon[points := [pt(0.0 - 4.125, 0.0 - 9.5), pt(2000000.5, 3.0), pt(nz, 0.1), pt(0.3, 0.7), pt(0.0 - 0.001, 5.0)], paint := p]
  out = [r1, r2, r3, r4, r5, r6, r7, r8, q1, q2, q3, q4, q5, c1, c2, c3, c4, c5, l1, l2, l3, l4, l5, l6, g1, g2, g3, g4, g5, g6, g7]
  RETURN out
END FUNC
"#;

/// Frame 1 is the whole native matrix. Frame 2 is items the native builder must
/// decline — a real transform, a two-stop gradient, an `Ellipse`, an `Arc` — so the
/// MFBASIC builders still run for them.
fn matrix_program() -> String {
    format!(
        "IMPORT app\nIMPORT canvas\nIMPORT color\nIMPORT collections\nIMPORT io\n\
         {PAINTS}{SHAPES}\n\
         SUB main()\n  \
         app::setMode(app::Mode.Canvas)\n  \
         MUT items AS List OF canvas::DrawItem = []\n  \
         FOR EACH p IN paints()\n    \
         items = collections::append(items, shapes(p))\n  \
         NEXT\n  \
         canvas::present(items)\n  \
         LET t AS canvas::Transform = canvas::Transform[a := 1.0, b := 0.0, c := 0.0, d := 1.0, tx := 3.5, ty := 0.0]\n  \
         LET stops AS List OF canvas::GradientStop = [canvas::GradientStop[offset := 0.0, color := color::rgb(255, 0, 0)], canvas::GradientStop[offset := 1.0, color := color::rgb(0, 0, 255)]]\n  \
         LET moved AS canvas::DrawItem = canvas::Rectangle[x := 1.0, y := 2.0, w := 3.0, h := 4.0, paint := WITH canvas::fill(color::rgb(1, 2, 3)) {{ transform := t }}]\n  \
         LET grad AS canvas::Gradient = canvas::Gradient[kind := canvas::GradientKind.Linear, startPoint := canvas::Point[x := 0.0, y := 0.0], endPoint := canvas::Point[x := 10.0, y := 0.0], stops := stops]\n  \
         LET gradPaint AS canvas::Paint = WITH canvas::fill(color::rgb(1, 2, 3)) {{ fillGradient := grad }}\n  \
         LET ramp AS canvas::DrawItem = canvas::Circle[x := 50.0, y := 50.0, radius := 9.0, paint := gradPaint]\n  \
         LET oval AS canvas::DrawItem = canvas::Ellipse[x := 40.0, y := 40.0, radiusX := 10.0, radiusY := 5.0, angle := 0.5, paint := canvas::fill(color::rgb(9, 9, 9))]\n  \
         LET bow AS canvas::DrawItem = canvas::Arc[x := 40.0, y := 40.0, radius := 10.0, startAngle := 0.0, endAngle := 2.0, cap := canvas::CapStyle.Round, paint := canvas::stroke(color::rgb(9, 9, 9), 2.0)]\n  \
         canvas::present([moved, ramp, oval, bow])\n  \
         io::print(\"rendered\")\n\
         END SUB\n"
    )
}

/// Every record the native builder produces for the matrix is bit-identical to the one
/// the MFBASIC builders produce, and the native builder is what built every item of it.
#[test]
fn the_native_geometry_matches_the_mfbasic_builders_bit_for_bit() {
    let lines = stats("canvas_geo_native_matrix", &matrix_program());
    assert_eq!(lines.len(), 2, "one frame per present: {lines:?}");
    let (matrix, declined) = (&lines[0], &lines[1]);

    let native = field(matrix, "geoNative");
    let built = field(matrix, "generations");
    assert!(
        native > 1000,
        "the matrix is ~6,000 items of the five native kinds (distinct ones: over a \
         thousand), but the native builder produced {native} records.\n{matrix}"
    );
    assert_eq!(
        native, built,
        "every item of the matrix is a native kind with an identity transform and no \
         gradient, so every geometry build must have been the native one.\n{matrix}"
    );
    assert_eq!(
        field(matrix, "geoVerified"),
        native,
        "with MFB_CANVAS_GEO_VERIFY=1 every native record is checked.\n{matrix}"
    );
    assert_eq!(
        field(matrix, "geoVerifyMismatches"),
        0,
        "a native geometry record differs from the MFBASIC builders' in at least one \
         bit. The software rasteriser reads this record and its goldens are exact, so \
         the two must agree slot for slot (`func_geo_build.rs`).\n{matrix}"
    );

    assert_eq!(
        field(declined, "geoNative"),
        native,
        "a transform, a two-stop gradient, an Ellipse and an Arc are the MFBASIC \
         builders' — the native builder must decline all four.\n{declined}"
    );
    assert_eq!(
        field(declined, "generations") - built,
        4,
        "the four declined items must still be built, by the MFBASIC path.\n{declined}"
    );
    assert_eq!(field(declined, "geoVerifyMismatches"), 0, "{declined}");
}
